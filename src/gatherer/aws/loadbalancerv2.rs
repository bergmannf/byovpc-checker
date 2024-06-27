use std::collections::HashMap;

use aws_sdk_ec2::types::{Filter, NetworkInterface};
use aws_sdk_ec2::Client;
use aws_sdk_elasticloadbalancingv2::operation::describe_load_balancers::DescribeLoadBalancersOutput;
use aws_sdk_elasticloadbalancingv2::types::LoadBalancer;
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use log::debug;

use crate::gatherer::aws::shared_types::{Collector, DefaultCollector, HypershiftCollector};
use crate::types::MinimalClusterInfo;

use super::shared_types::AWSLoadBalancer;

pub async fn get_load_balancers(
    elb_client: &ELBv2Client,
    cluster_info: &MinimalClusterInfo,
) -> Result<Vec<LoadBalancer>, aws_sdk_elasticloadbalancingv2::Error> {
    debug!("Retrieving LoadBalancers");
    let mut lb_arns = HashMap::new();
    let collector: Box<dyn Collector + Send> = match cluster_info.cluster_type {
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
    let mut cluster_lbs = vec![];
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
                        if collector.match_tag(t.into()) {
                            debug!("Tag matched");
                            cluster_lbs.push(lb_val.clone())
                        }
                    }
                }
            }
        }
    }
    Ok(cluster_lbs)
}

pub async fn get_load_balancer_enis(
    ec2_client: &Client,
    lbs: &Vec<AWSLoadBalancer>,
) -> Result<Vec<NetworkInterface>, aws_sdk_ec2::Error> {
    debug!("Retrieving ENIs for LoadBalancers");
    let network_interfaces;
    // aws ec2 describe-network-interfaces --filters Name=description,Values="ELB $MC_LB_NAME" --query 'NetworkInterfaces[].PrivateIpAddresses[].PrivateIpAddress' --no-cli-pager --output yaml >> "$TMP_FILE"
    let descriptions: Vec<String> = lbs
        .iter()
        .map(|lb| match &lb {
            &AWSLoadBalancer::ClassicLoadBalancer(lb) => lb
                .load_balancer_name()
                .as_ref()
                .map_or("".to_string(), |n| format!("ELB {}", n)),
            &AWSLoadBalancer::ModernLoadBalancer(lb) => lb
                .load_balancer_name()
                .as_ref()
                .map_or("".to_string(), |n| format!("ELB {}", n)),
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
