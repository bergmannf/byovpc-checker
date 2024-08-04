pub mod conversion;
pub mod ec2;
pub mod loadbalancer;
pub mod loadbalancerv2;
pub mod shared_types;

use crate::gatherer::aws::conversion::{AWSLoadBalancer, SubnetProxy};
pub use crate::gatherer::aws::loadbalancer::get_classic_load_balancers;
use crate::types::MinimalClusterInfo;

use crate::gatherer::Gatherer;
use aws_config::meta::region::RegionProviderChain;
use aws_config::BehaviorVersion;
use aws_config::SdkConfig;
use aws_sdk_ec2::Client as EC2Client;
use aws_sdk_elasticloadbalancing::Client as ELBv1Client;
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use headers::Authorization;
use hyper::client::HttpConnector;
use hyper::Uri;
use hyper_proxy::{Intercept, Proxy, ProxyConnector};
use log::debug;
use log::error;
use log::info;
use url::Url;

/// Struct that holds all data available in AWS once we gathered it.
pub struct AWSClusterData {
    pub subnets: Vec<SubnetProxy>,
    pub routetables: Vec<aws_sdk_ec2::types::RouteTable>,
    pub load_balancers: Vec<aws_sdk_elasticloadbalancingv2::types::LoadBalancer>,
    pub classic_load_balancers: Vec<aws_sdk_elasticloadbalancing::types::LoadBalancerDescription>,
    pub load_balancer_enis: Vec<aws_sdk_ec2::types::NetworkInterface>,
    pub instances: Vec<aws_sdk_ec2::types::Instance>,
}

/// Returns `ProxyConnector<HttpConnector>` if env. variable 'https_proxy' is set
pub fn determine_proxy() -> Option<ProxyConnector<HttpConnector>> {
    let proxy_url: Url = std::env::var("HTTPS_PROXY")
        .or_else(|_v| std::env::var("https_proxy"))
        .ok()?
        .parse()
        .ok()?;
    let mut proxy_uri: Uri = std::env::var("HTTPS_PROXY")
        .or_else(|_v| std::env::var("https_proxy"))
        .ok()?
        .parse()
        .ok()?;
    if proxy_uri.scheme().is_none() {
        error!("Configured proxy did not specify a scheme - falling back to HTTP.");
        proxy_uri = format!("http://{}", std::env::var("HTTPS_PROXY").ok()?)
            .parse()
            .ok()?;
    }
    let mut proxy = Proxy::new(Intercept::All, proxy_uri);

    if let Some(password) = proxy_url.password() {
        proxy.set_authorization(Authorization::basic(proxy_url.username(), password));
    }

    let connector = HttpConnector::new();
    Some(ProxyConnector::from_proxy(connector, proxy).unwrap())
}

/// Will setup the SdkConfig with a proxy if needed.
pub async fn aws_setup() -> SdkConfig {
    let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
    debug!("Using region: {}", region_provider.region().await.unwrap());
    let config = if let Some(proxy) = determine_proxy() {
        debug!("Using proxy");
        let client =
            aws_smithy_runtime::client::http::hyper_014::HyperClientBuilder::new().build(proxy);
        aws_config::defaults(BehaviorVersion::latest())
            .region(region_provider)
            .load()
            .await
            .into_builder()
            .http_client(client.clone())
            .build()
    } else {
        debug!("Not using a proxy");
        aws_config::defaults(BehaviorVersion::latest())
            .region(region_provider)
            .load()
            .await
    };
    return config;
}

/// Gathers all required data associated with the cluster from AWS.
pub async fn gather(cluster_info: &MinimalClusterInfo) -> AWSClusterData {
    let aws_config = crate::gatherer::aws::aws_setup().await;

    let ec2_client = EC2Client::new(&aws_config);
    let elbv2_client = ELBv2Client::new(&aws_config);
    let elbv1_client = ELBv1Client::new(&aws_config);

    info!("Fetching LoadBalancer data");
    let h1 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            info!("Fetching load balancers");
            let lbs = crate::gatherer::aws::loadbalancerv2::LoadBalancerGatherer {
                client: &elbv2_client,
                cluster_info: &cluster_info,
            }
            .gather()
            .await
            .expect("could not retrieve load balancers");
            let classic_lbs =
                crate::gatherer::aws::get_classic_load_balancers(&elbv1_client, &cluster_info)
                    .await
                    .expect("could not retrieve classic load balancers");
            let ec2_client = ec2_client.clone();
            let lbs = lbs.clone();
            let mut mlbs: Vec<AWSLoadBalancer> = lbs
                .clone()
                .into_iter()
                .map(|l| AWSLoadBalancer::ModernLoadBalancer(l))
                .collect();
            let mut clbs: Vec<AWSLoadBalancer> = classic_lbs
                .clone()
                .into_iter()
                .map(|l| AWSLoadBalancer::ClassicLoadBalancer(l))
                .collect();
            clbs.append(&mut mlbs);
            let enig = crate::gatherer::aws::ec2::NetworkInterfaceGatherer {
                client: &ec2_client,
                loadbalancers: &clbs,
            };
            let eni_lbs = enig.gather().await.expect("could not retrieve ENIs");
            (lbs, classic_lbs, eni_lbs)
        }
    });

    info!("Fetching Subnet data");
    let h2 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            let sg = crate::gatherer::aws::ec2::ConfiguredSubnetGatherer {
                client: &ec2_client,
                cluster_info: &cluster_info,
            };
            let all_subnets = sg
                .gather()
                .await
                .expect("Could not retrieve configured subnets");
            let subnet_ids = all_subnets
                .iter()
                .map(|s| s.subnet_id.as_ref().unwrap().clone())
                .collect();
            info!("Fetching all routetables");
            let rtg = crate::gatherer::aws::ec2::RouteTableGatherer {
                client: &ec2_client,
                subnet_ids: &subnet_ids,
            };
            let routetables = rtg.gather().await.expect("Could not retrieve routetables");
            (all_subnets, routetables)
        }
    });

    info!("Fetching instances and security groups");
    let h3 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            let instances = crate::gatherer::aws::ec2::InstanceGatherer {
                client: &ec2_client,
                cluster_info: &cluster_info,
            }
            .gather()
            .await
            .expect("Could not retrieve instances");
            instances
        }
    });

    let (load_balancers, classic_load_balancers, load_balancer_enis) = h1.await.unwrap();
    let (subnets, routetables) = h2.await.unwrap();
    let instances = h3.await.unwrap();

    AWSClusterData {
        subnets: subnets
            .iter()
            .map(|s| SubnetProxy::from(s.clone()))
            .collect(),
        routetables,
        load_balancers,
        classic_load_balancers,
        load_balancer_enis,
        instances,
    }
}
