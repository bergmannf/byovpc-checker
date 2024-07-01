pub mod ec2;
pub mod loadbalancer;
pub mod loadbalancerv2;
pub mod shared_types;

pub use crate::gatherer::aws::ec2::get_load_balancer_enis;
pub use crate::gatherer::aws::loadbalancer::get_classic_load_balancers;
pub use crate::gatherer::aws::loadbalancerv2::get_load_balancers;

use aws_config::meta::region::RegionProviderChain;
use aws_config::BehaviorVersion;
use aws_config::SdkConfig;
use headers::Authorization;
use hyper::client::HttpConnector;
use hyper::Uri;
use hyper_proxy::{Intercept, Proxy, ProxyConnector};
use log::debug;
use log::error;
use url::Url;

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
