use std::error::Error;
use std::ops::Deref;

use aws_config::BehaviorVersion;
use aws_config::SdkConfig;
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_ec2::Client;
use aws_sdk_ec2::error::SdkError;
use aws_sdk_ec2::operation::describe_route_tables::DescribeRouteTablesError;
use aws_sdk_ec2::operation::describe_route_tables::DescribeRouteTablesOutput;
use aws_sdk_ec2::operation::describe_subnets::DescribeSubnetsError;
use aws_sdk_ec2::operation::describe_subnets::DescribeSubnetsOutput;
use aws_sdk_ec2::types::Filter;
use aws_sdk_ec2::types::Subnet;
use log::info;

use crate::types::InvariantError;
use crate::types::MinimalClusterInfo;

pub async fn aws_setup() -> SdkConfig {
    let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    return config
}

pub async fn get_subnets(ec2_client: &Client, subnet_ids: &Vec<String>) -> Result<DescribeSubnetsOutput, SdkError<DescribeSubnetsError>> {
    ec2_client.describe_subnets().set_subnet_ids(Some(subnet_ids.clone())).send().await
}

pub async fn get_all_subnets(ec2_client: &Client, configured_subnets: &Vec<Subnet>) -> Result<Vec<Subnet>, Box<dyn Error>>{
    let mut vpc_ids: Vec<&String> = configured_subnets.iter().map(|s| s.vpc_id.as_ref().unwrap()).collect();
    vpc_ids.dedup();
    if vpc_ids.len() > 1 {
        return Err(Box::new(InvariantError{msg: format!("More than 1 VPC found associated with cluster subnets: {:?}", vpc_ids)}))
    }
    let aws_subnets_by_vpc = get_subnets_by_vpc(&ec2_client, vpc_ids[0].as_str()).await;
    let aws_unwrapped_subnets_by_vpc = aws_subnets_by_vpc.unwrap().subnets.unwrap();
    return Ok(aws_unwrapped_subnets_by_vpc)
}

pub async fn get_subnets_by_vpc(ec2_client: &Client, vpc_id: &str) -> Result<DescribeSubnetsOutput, SdkError<DescribeSubnetsError>> {
    info!("Retrieving subnets for VPC: {}", vpc_id);
    let subnets_filter = Filter::builder().
        name("vpc-id").
        values(vpc_id).
        build();
    return ec2_client.describe_subnets().set_filters(Some(vec![subnets_filter])).send().await;
}

pub async fn get_route_tables(ec2_client: &Client, subnet_ids: &Vec<String>) -> Result<DescribeRouteTablesOutput, SdkError<DescribeRouteTablesError>> {
    info!("Retrieving route tables for subnets: {}", subnet_ids.join(","));
    let rtb_filter = Filter::builder().
        name("association.subnet-id").
        set_values(Some(subnet_ids.clone())).
        build();
    return ec2_client.describe_route_tables().set_filters(Some(vec![rtb_filter])).send().await;
}

pub async fn get_load_balancer(ec2_client: &Client, cluster_info: &MinimalClusterInfo) {
    panic!("not implemented")
}

pub async fn get_security_groups(ec2_client: &Client, cluster_info: &MinimalClusterInfo) {
    panic!("not implemented")
}
