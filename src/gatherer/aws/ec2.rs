use aws_sdk_ec2::{
    types::{
        Filter, GroupIdentifier, Instance, NetworkInterface, RouteTable, SecurityGroup, Subnet,
    },
    Client,
};
use log::{debug, info};
use std::error::Error;

use crate::types::{InvariantError, MinimalClusterInfo};

use super::shared_types::{AWSLoadBalancer, CLUSTER_TAG_PREFIX};

pub async fn get_subnets(
    ec2_client: &Client,
    cluster_info: &MinimalClusterInfo,
) -> Result<Vec<Subnet>, aws_sdk_ec2::Error> {
    let cluster_name_tag = format!("{}{}", CLUSTER_TAG_PREFIX, cluster_info.cluster_infra_name);
    if !cluster_info.subnets.is_empty() {
        info!("Fetching subnets via IDs");
        match ec2_client
            .describe_subnets()
            .set_subnet_ids(Some(cluster_info.subnets.clone()))
            .send()
            .await
        {
            Ok(success) => Ok(success.subnets.unwrap()),
            Err(err) => Err(aws_sdk_ec2::Error::from(err)),
        }
    } else {
        info!("Fetching subnets via tags");
        match ec2_client
            .describe_subnets()
            .filters(
                Filter::builder()
                    .name("tag-key")
                    .values(cluster_name_tag)
                    .build(),
            )
            .send()
            .await
        {
            Ok(success) => Ok(success.subnets.unwrap()),
            Err(err) => Err(aws_sdk_ec2::Error::from(err)),
        }
    }
}

pub async fn get_all_subnets(
    ec2_client: &Client,
    cluster_info: &MinimalClusterInfo,
    configured_subnets: &Vec<Subnet>,
) -> Result<Vec<Subnet>, Box<dyn Error>> {
    debug!("Retrieving subnets");
    let vpcs;
    let vpc_ids = if configured_subnets.len() > 0 {
        debug!("Using configured subnets");
        let mut vpc_ids: Vec<&String> = configured_subnets
            .iter()
            .map(|s| s.vpc_id.as_ref().unwrap())
            .collect();
        vpc_ids.dedup();
        vpc_ids
    } else {
        debug!("Retrieving all VPCs tagged for cluster");
        let cluster_tag = format!(
            "tag:{}{}",
            CLUSTER_TAG_PREFIX, cluster_info.cluster_infra_name
        );
        let vpc_res = ec2_client
            .describe_vpcs()
            .filters(Filter::builder().name(cluster_tag).values("owned").build())
            .send()
            .await;
        vpcs = vpc_res
            .expect("could not retrieve VPCs by tag")
            .vpcs
            .unwrap();
        vpcs.iter().map(|v| v.vpc_id.as_ref().unwrap()).collect()
    };
    if vpc_ids.len() != 1 {
        return Err(Box::new(InvariantError {
            msg: format!(
                "Invalid number of VPCs found associated with cluster: {:?}",
                vpc_ids.len()
            ),
        }));
    }
    let aws_subnets_by_vpc = get_subnets_by_vpc(&ec2_client, vpc_ids[0].as_str()).await;
    let aws_unwrapped_subnets_by_vpc = aws_subnets_by_vpc.unwrap();
    return Ok(aws_unwrapped_subnets_by_vpc);
}

pub async fn get_subnets_by_vpc(
    ec2_client: &Client,
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
    ec2_client: &Client,
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

/// Returns the instances in this account with a matching cluster tag.
pub async fn get_instances(
    ec2_client: &Client,
    cluster_info: &MinimalClusterInfo,
) -> Result<Vec<Instance>, aws_sdk_ec2::Error> {
    let cluster_tag = format!(
        "tag:{}{}",
        CLUSTER_TAG_PREFIX, cluster_info.cluster_infra_name
    );
    let openshift_instances;
    match ec2_client
        .describe_instances()
        .filters(Filter::builder().name(cluster_tag).values("owned").build())
        .send()
        .await
    {
        Ok(instance_output) => {
            openshift_instances = instance_output
                .reservations
                .expect("Expected reservations to bet set")
                .into_iter()
                .map(|r| r.instances.unwrap())
                .flatten()
                .collect()
        }
        Err(err) => return Err(aws_sdk_ec2::Error::from(err)),
    }
    Ok(openshift_instances)
}

/// Returns the security groups in use by instances of the cluster.
pub async fn get_security_groups(
    ec2_client: &Client,
    cluster_info: &MinimalClusterInfo,
) -> Result<Vec<SecurityGroup>, aws_sdk_ec2::Error> {
    let instances = get_instances(ec2_client, cluster_info).await;
    let instance_security_groups = if let Ok(is) = instances {
        let mut sgs: Vec<GroupIdentifier> = is
            .into_iter()
            .map(|i| i.security_groups.unwrap())
            .flatten()
            .collect();
        sgs.dedup();
        ec2_client
            .describe_security_groups()
            .set_group_ids(Some(
                sgs.into_iter().map(|sg| sg.group_id.unwrap()).collect(),
            ))
            .send()
            .await
    } else {
        return Err(instances.err().unwrap());
    };
    match instance_security_groups {
        Ok(sg) => return Ok(sg.security_groups.unwrap()),
        Err(e) => return Err(aws_sdk_ec2::Error::from(e)),
    }
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
