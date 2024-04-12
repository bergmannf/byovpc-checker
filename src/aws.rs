use std::collections::HashMap;
use std::error::Error;

use aws_config::BehaviorVersion;
use aws_config::SdkConfig;
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_ec2::Client as EC2Client;
use aws_sdk_ec2::error::SdkError;
use aws_sdk_ec2::operation::describe_route_tables::DescribeRouteTablesError;
use aws_sdk_ec2::operation::describe_route_tables::DescribeRouteTablesOutput;
use aws_sdk_ec2::operation::describe_subnets::DescribeSubnetsError;
use aws_sdk_ec2::operation::describe_subnets::DescribeSubnetsOutput;
use aws_sdk_ec2::types::Filter;
use aws_sdk_ec2::types::NetworkInterface;
use aws_sdk_ec2::types::Subnet;
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use aws_sdk_elasticloadbalancingv2::operation::describe_load_balancers::DescribeLoadBalancersOutput;
use aws_sdk_elasticloadbalancingv2::types::LoadBalancer;
use aws_sdk_elasticloadbalancingv2::types::Tag;
use log::debug;
use log::info;

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

struct DefaultCollector {
    clusterid: String,
}

impl Collector for HypershiftCollector {
    fn match_tag(&self, t: Tag) -> bool {
        t.key.is_some_and(|t| t == DEFAULT_ROUTER_TAG_HYPERSHIFT) && t.value.is_some_and(|t| t == DEFAULT_ROUTER_VALUE_HYPERSHIFT)
    }
}

impl Collector for DefaultCollector {
    fn match_tag(&self, t: Tag) -> bool {
        let cluster_tag = format!("{}{}", CLUSTER_TAG_PREFIX, self.clusterid);
        t.key.is_some_and(|t| t.contains(&DEFAULT_ROUTER_TAG.to_string()) && t.contains(&cluster_tag))
    }
}

pub async fn aws_setup() -> SdkConfig {
    let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    return config
}

pub async fn get_subnets(ec2_client: &EC2Client, subnet_ids: &Vec<String>) -> Result<DescribeSubnetsOutput, SdkError<DescribeSubnetsError>> {
    ec2_client.describe_subnets().set_subnet_ids(Some(subnet_ids.clone())).send().await
}

pub async fn get_all_subnets(ec2_client: &EC2Client, configured_subnets: &Vec<Subnet>) -> Result<Vec<Subnet>, Box<dyn Error>>{
    let mut vpc_ids: Vec<&String> = configured_subnets.iter().map(|s| s.vpc_id.as_ref().unwrap()).collect();
    vpc_ids.dedup();
    if vpc_ids.len() > 1 {
        return Err(Box::new(InvariantError{msg: format!("More than 1 VPC found associated with cluster subnets: {:?}", vpc_ids)}))
    }
    let aws_subnets_by_vpc = get_subnets_by_vpc(&ec2_client, vpc_ids[0].as_str()).await;
    let aws_unwrapped_subnets_by_vpc = aws_subnets_by_vpc.unwrap().subnets.unwrap();
    return Ok(aws_unwrapped_subnets_by_vpc)
}

pub async fn get_subnets_by_vpc(ec2_client: &EC2Client, vpc_id: &str) -> Result<DescribeSubnetsOutput, SdkError<DescribeSubnetsError>> {
    info!("Retrieving subnets for VPC: {}", vpc_id);
    let subnets_filter = Filter::builder().
        name("vpc-id").
        values(vpc_id).
        build();
    return ec2_client.describe_subnets().set_filters(Some(vec![subnets_filter])).send().await;
}

pub async fn get_route_tables(ec2_client: &EC2Client, subnet_ids: &Vec<String>) -> Result<DescribeRouteTablesOutput, SdkError<DescribeRouteTablesError>> {
    info!("Retrieving route tables for subnets: {}", subnet_ids.join(","));
    let rtb_filter = Filter::builder().
        name("association.subnet-id").
        set_values(Some(subnet_ids.clone())).
        build();
    return ec2_client.describe_route_tables().set_filters(Some(vec![rtb_filter])).send().await;
}

pub async fn get_load_balancers(elb_client: &ELBv2Client, cluster_info: &MinimalClusterInfo) -> Result<Vec<LoadBalancer>, aws_sdk_elasticloadbalancingv2::Error>{
    let mut lb_arns = HashMap::new();
    let collector: Box<dyn Collector> = match cluster_info.cluster_type {
        crate::types::ClusterType::Hypershift => Box::new(HypershiftCollector{}),
        _ => Box::new(DefaultCollector{clusterid: cluster_info.cluster_id.clone()}),
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
        match elb_client.describe_tags().resource_arns(lb_key).send().await{
            Ok(success) => tags = success,
            Err(err) => return Err(aws_sdk_elasticloadbalancingv2::Error::from(err)),
        };
        if let Some(tag_descriptions) = tags.tag_descriptions {
            for td in tag_descriptions {
                if let Some(tag) = td.tags {
                    debug!("Checking tag: {:?}", tag);
                    for t in tag {
                        if collector.match_tag(t) {
                            default_router_lbs.push(lb_val.clone())
                        }
                    }
                }
            }
        }
    }
    Ok(default_router_lbs)
}

pub async fn get_load_balancer_enis(ec2_client: &EC2Client, lbs: Vec<LoadBalancer>) -> Result<Vec<NetworkInterface>, aws_sdk_ec2::Error> {
    let network_interfaces;
    // aws ec2 describe-network-interfaces --filters Name=description,Values="ELB $MC_LB_NAME" --query 'NetworkInterfaces[].PrivateIpAddresses[].PrivateIpAddress' --no-cli-pager --output yaml >> "$TMP_FILE"
    let descriptions: Vec<String> = lbs.iter()
                          .map(|lb| lb.load_balancer_name
                               .as_ref()
                               .map_or("".to_string(), |n| format!("ELB {}", n)))
                          .collect();
    let result = ec2_client
        .describe_network_interfaces()
        .filters(Filter::builder()
                 .name("description")
                 .values(descriptions.join(","))
                 .build())
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
