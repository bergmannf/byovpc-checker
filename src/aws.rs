use std::collections::HashMap;
use std::error::Error;

use aws_config::meta::region::RegionProviderChain;
use aws_config::BehaviorVersion;
use aws_config::SdkConfig;
use aws_sdk_ec2::types::Filter;
use aws_sdk_ec2::types::NetworkInterface;
use aws_sdk_ec2::types::RouteTable;
use aws_sdk_ec2::types::Subnet;
use aws_sdk_ec2::Client as EC2Client;
use aws_sdk_elasticloadbalancingv2::operation::describe_load_balancers::DescribeLoadBalancersOutput;
use aws_sdk_elasticloadbalancingv2::types::LoadBalancer;
use aws_sdk_elasticloadbalancingv2::types::Tag;
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use headers::Authorization;
use hyper::client::HttpConnector;
use hyper::Uri;
use hyper_proxy::{Intercept, Proxy, ProxyConnector};
use log::debug;
use log::error;
use url::Url;

use crate::types::InvariantError;
use crate::types::MinimalClusterInfo;

pub const DEFAULT_ROUTER_TAG_HYPERSHIFT: &str = "kubernetes.io/service-name";
pub const DEFAULT_ROUTER_VALUE_HYPERSHIFT: &str = "openshift-ingress/router-default";
pub const DEFAULT_ROUTER_TAG: &str = "openshift-ingress/router-default";
pub const CLUSTER_TAG_PREFIX: &str = "kubernetes.io/cluster/";

trait Collector {
    fn match_tag(&self, t: Tag) -> bool;
}

struct HypershiftCollector;

struct DefaultCollector<'a> {
    cluster_id: &'a String,
    cluster_infra_name: &'a String,
}

impl Collector for HypershiftCollector {
    fn match_tag(&self, t: Tag) -> bool {
        t.key.is_some_and(|t| t == DEFAULT_ROUTER_TAG_HYPERSHIFT)
            && t.value
                .is_some_and(|t| t == DEFAULT_ROUTER_VALUE_HYPERSHIFT)
    }
}

impl<'a> Collector for DefaultCollector<'a> {
    fn match_tag(&self, t: Tag) -> bool {
        let cluster_id_tag = format!("{}{}", CLUSTER_TAG_PREFIX, self.cluster_id);
        let cluster_name_tag = format!("{}{}", CLUSTER_TAG_PREFIX, self.cluster_infra_name);
        debug!(
            "Checking if {:?} matches {} or {}",
            t, cluster_id_tag, cluster_name_tag
        );
        t.key
            .is_some_and(|t| t.contains(&cluster_id_tag) || t.contains(&cluster_name_tag))
            && t.value.is_some_and(|t| t == "owned" || t == "shared")
    }
}

/// Returns `ProxyConnector<HttpConnector>` if env. variable 'https_proxy' is set
pub fn determine_proxy() -> Option<ProxyConnector<HttpConnector>> {
    let proxy_url: Url = std::env::var("HTTPS_PROXY").ok()?.parse().ok()?;
    let mut proxy_uri: Uri = std::env::var("HTTPS_PROXY").ok()?.parse().ok()?;
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

pub async fn aws_setup() -> SdkConfig {
    let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
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

pub async fn get_subnets(
    ec2_client: &EC2Client,
    subnet_ids: &Vec<String>,
) -> Result<Vec<Subnet>, aws_sdk_ec2::Error> {
    match ec2_client
        .describe_subnets()
        .set_subnet_ids(Some(subnet_ids.clone()))
        .send()
        .await
    {
        Ok(success) => Ok(success.subnets.unwrap()),
        Err(err) => Err(aws_sdk_ec2::Error::from(err)),
    }
}

pub async fn get_all_subnets(
    ec2_client: &EC2Client,
    configured_subnets: &Vec<Subnet>,
) -> Result<Vec<Subnet>, Box<dyn Error>> {
    debug!("Retrieving subnets");
    let mut vpc_ids: Vec<&String> = configured_subnets
        .iter()
        .map(|s| s.vpc_id.as_ref().unwrap())
        .collect();
    vpc_ids.dedup();
    if vpc_ids.len() > 1 {
        return Err(Box::new(InvariantError {
            msg: format!(
                "More than 1 VPC found associated with cluster subnets: {:?}",
                vpc_ids
            ),
        }));
    }
    let aws_subnets_by_vpc = get_subnets_by_vpc(&ec2_client, vpc_ids[0].as_str()).await;
    let aws_unwrapped_subnets_by_vpc = aws_subnets_by_vpc.unwrap();
    return Ok(aws_unwrapped_subnets_by_vpc);
}

pub async fn get_subnets_by_vpc(
    ec2_client: &EC2Client,
    vpc_id: &str,
) -> Result<Vec<Subnet>, aws_sdk_ec2::Error> {
    debug!("Retrieving subnets for VPC: {}", vpc_id);
    let subnets_filter = Filter::builder().name("vpc-id").values(vpc_id).build();
    match ec2_client
        .describe_subnets()
        .set_filters(Some(vec![subnets_filter]))
        .send()
        .await
    {
        Ok(success) => Ok(success.subnets.unwrap()),
        Err(err) => Err(aws_sdk_ec2::Error::from(err)),
    }
}

pub async fn get_route_tables(
    ec2_client: &EC2Client,
    subnet_ids: &Vec<String>,
) -> Result<Vec<RouteTable>, aws_sdk_ec2::Error> {
    debug!(
        "Retrieving route tables for subnets: {}",
        subnet_ids.join(",")
    );
    let rtb_filter = Filter::builder()
        .name("association.subnet-id")
        .set_values(Some(subnet_ids.clone()))
        .build();
    match ec2_client
        .describe_route_tables()
        .set_filters(Some(vec![rtb_filter]))
        .send()
        .await
    {
        Ok(success) => Ok(success.route_tables.unwrap()),
        Err(err) => Err(aws_sdk_ec2::Error::from(err)),
    }
}

pub async fn get_load_balancers(
    elb_client: &ELBv2Client,
    cluster_info: &MinimalClusterInfo,
) -> Result<Vec<LoadBalancer>, aws_sdk_elasticloadbalancingv2::Error> {
    debug!("Retrieving LoadBalancers");
    let mut lb_arns = HashMap::new();
    let collector: Box<dyn Collector> = match cluster_info.cluster_type {
        crate::types::ClusterType::Hypershift => {
            debug!("Using hypershift collector");
            Box::new(HypershiftCollector {})
        }
        _ => {
            debug!("Using default collector");
            Box::new(DefaultCollector {
                cluster_id: &cluster_info.cluster_id,
                cluster_infra_name: &cluster_info.cluster_infra_name,
            })
        }
    };
    let mut default_router_lbs = vec![];
    let lb_out: DescribeLoadBalancersOutput;
    match elb_client.describe_load_balancers().send().await {
        Ok(success) => lb_out = success,
        Err(err) => return Err(aws_sdk_elasticloadbalancingv2::Error::from(err)),
    };
    if let Some(lbs) = lb_out.load_balancers {
        for lb in lbs {
            let arn = lb.load_balancer_arn.as_ref().unwrap().clone();
            lb_arns.insert(arn, lb);
        }
    }
    for (lb_key, lb_val) in lb_arns {
        debug!("Checking loadbalancer: {}", lb_key);
        let tags;
        match elb_client
            .describe_tags()
            .resource_arns(lb_key)
            .send()
            .await
        {
            Ok(success) => tags = success,
            Err(err) => return Err(aws_sdk_elasticloadbalancingv2::Error::from(err)),
        };
        if let Some(tag_descriptions) = tags.tag_descriptions {
            for td in tag_descriptions {
                if let Some(tag) = td.tags {
                    for t in tag {
                        debug!("Checking tag: {:?}", t);
                        if collector.match_tag(t) {
                            debug!("Tag matched");
                            default_router_lbs.push(lb_val.clone())
                        }
                    }
                }
            }
        }
    }
    Ok(default_router_lbs)
}

pub async fn get_load_balancer_enis(
    ec2_client: &EC2Client,
    lbs: &Vec<LoadBalancer>,
) -> Result<Vec<NetworkInterface>, aws_sdk_ec2::Error> {
    debug!("Retrieving ENIs for LoadBalancers");
    let network_interfaces;
    // aws ec2 describe-network-interfaces --filters Name=description,Values="ELB $MC_LB_NAME" --query 'NetworkInterfaces[].PrivateIpAddresses[].PrivateIpAddress' --no-cli-pager --output yaml >> "$TMP_FILE"
    let descriptions: Vec<String> = lbs
        .iter()
        .map(|lb| {
            lb.load_balancer_name
                .as_ref()
                .map_or("".to_string(), |n| format!("ELB {}", n))
        })
        .collect();
    let result = ec2_client
        .describe_network_interfaces()
        .filters(
            Filter::builder()
                .name("description")
                .values(descriptions.join(","))
                .build(),
        )
        .send()
        .await;
    match result {
        Ok(success) => network_interfaces = success.network_interfaces,
        Err(err) => return Err(aws_sdk_ec2::Error::from(err)),
    }
    Ok(network_interfaces.unwrap())
}

pub async fn get_security_groups(ec2_client: &EC2Client, cluster_info: &MinimalClusterInfo) {
    panic!("not implemented")
}
