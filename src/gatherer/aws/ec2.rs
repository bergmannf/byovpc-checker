use async_trait::async_trait;
use aws_sdk_ec2::{
    types::{
        Filter, GroupIdentifier, Instance, NetworkInterface, RouteTable, SecurityGroup, Subnet,
    },
    Client,
};
use itertools::Itertools;
use log::{debug, error, info};
use std::error::Error;

use crate::gatherer::Gatherer;
use crate::types::{InvariantError, MinimalClusterInfo};

use super::shared_types::{AWSInstance, AWSLoadBalancer, CLUSTER_TAG_PREFIX};

/// Retrieves the subnets
/// This gatherer will retrieve:
/// - All configured subnets
/// - All subnets in the same VPC as the configured subnets
/// - All subnets tagged for the cluster
pub struct ConfiguredSubnetGatherer<'a> {
    pub client: &'a Client,
    pub cluster_info: &'a MinimalClusterInfo,
}

impl<'a> ConfiguredSubnetGatherer<'a> {
    async fn get_subnets_configured(&self) -> Result<Vec<Subnet>, Box<dyn Error>> {
        info!("Fetching subnets via IDs");
        if !self.cluster_info.subnets.is_empty() {
            match self
                .client
                .describe_subnets()
                .set_subnet_ids(Some(self.cluster_info.subnets.clone()))
                .send()
                .await
            {
                Ok(success) => Ok(success.subnets.unwrap()),
                Err(err) => {
                    error!("Failed to fetch configured subnets: {}", err);
                    Err(Box::new(err))
                }
            }
        } else {
            Ok(vec![])
        }
    }

    async fn get_subnets_by_tag(&self) -> Result<Vec<Subnet>, Box<dyn Error>> {
        let cluster_name_tag = format!(
            "{}{}",
            CLUSTER_TAG_PREFIX, self.cluster_info.cluster_infra_name
        );
        info!("Fetching subnets via tags");
        match self
            .client
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
            Err(err) => {
                error!("Failed to fetch subnets by tags: {}", err);
                Err(Box::new(err))
            }
        }
    }

    async fn get_subnets_by_vpc(&self, vpcid: String) -> Result<Vec<Subnet>, Box<dyn Error>> {
        debug!("Retrieving subnets for VPC: {}", vpcid);
        let subnets_filter = Filter::builder().name("vpc-id").values(vpcid).build();
        match self
            .client
            .describe_subnets()
            .set_filters(Some(vec![subnets_filter]))
            .send()
            .await
        {
            Ok(success) => Ok(success.subnets.unwrap()),
            Err(err) => {
                error!("Failed to fetch subnets by VPCID: {}", err);
                Err(Box::new(err))
            }
        }
    }
}

#[async_trait]
impl<'a> Gatherer for ConfiguredSubnetGatherer<'a> {
    type Resource = Subnet;

    async fn gather(&self) -> Result<Vec<Subnet>, Box<dyn Error>> {
        let mut all_subnets = vec![];
        {
            match self.get_subnets_configured().await {
                Ok(ref s) => all_subnets.extend(s.clone()),
                Err(_) => {}
            }
        }
        {
            match self.get_subnets_by_tag().await {
                Ok(ref s) => all_subnets.extend(s.clone()),
                Err(_) => {}
            }
        }
        let vpcid = {
            debug!("Using configured subnets");
            let mut vpc_ids: Vec<&String> = all_subnets
                .iter()
                .map(|s| s.vpc_id.as_ref().unwrap())
                .collect();
            vpc_ids.dedup();
            (**vpc_ids.first().unwrap()).clone()
        };
        match self.get_subnets_by_vpc(vpcid).await {
            Ok(ref s) => {
                all_subnets.extend(s.clone());
                let result: Vec<_> = all_subnets
                    .clone()
                    .into_iter()
                    .unique_by(|s| s.clone().subnet_id)
                    .collect();
                Ok(result)
            }
            Err(e) => Err(e),
        }
    }
}

/// Gather the routetables associated with the subnets.
pub struct RouteTableGatherer<'a> {
    pub client: &'a Client,
    pub subnet_ids: &'a Vec<String>,
}

#[async_trait]
impl<'a> Gatherer for RouteTableGatherer<'a> {
    type Resource = RouteTable;

    async fn gather(&self) -> Result<Vec<Self::Resource>, Box<dyn Error>> {
        debug!(
            "Retrieving route tables for subnets: {}",
            self.subnet_ids.join(",")
        );
        let rtb_filter = Filter::builder()
            .name("association.subnet-id")
            .set_values(Some(self.subnet_ids.clone()))
            .build();
        match self
            .client
            .describe_route_tables()
            .set_filters(Some(vec![rtb_filter]))
            .send()
            .await
        {
            Ok(success) => Ok(success.route_tables.unwrap()),
            Err(err) => Err(Box::new(err)),
        }
    }
}

pub struct InstanceGatherer<'a> {
    pub client: &'a Client,
    pub cluster_info: &'a MinimalClusterInfo,
}

impl<'a> InstanceGatherer<'a> {
    /// Returns the security groups in use by instances of the cluster.
    pub async fn get_security_groups(
        &self,
        instances: &Vec<Instance>,
    ) -> Result<Vec<SecurityGroup>, Box<dyn Error>> {
        let mut sgs: Vec<GroupIdentifier> = instances
            .into_iter()
            .map(|i| i.security_groups.clone().unwrap())
            .flatten()
            .collect();
        sgs.dedup();
        let instance_security_groups = self
            .client
            .describe_security_groups()
            .set_group_ids(Some(
                sgs.into_iter().map(|sg| sg.group_id.unwrap()).collect(),
            ))
            .send()
            .await;
        match instance_security_groups {
            Ok(sg) => return Ok(sg.security_groups.unwrap()),
            Err(e) => return Err(Box::new(e)),
        }
    }
}

#[async_trait]
impl<'a> Gatherer for InstanceGatherer<'a> {
    type Resource = Instance;

    async fn gather(&self) -> Result<Vec<Self::Resource>, Box<dyn Error>> {
        let cluster_tag = format!(
            "tag:{}{}",
            CLUSTER_TAG_PREFIX, self.cluster_info.cluster_infra_name
        );
        let openshift_instances;
        match self
            .client
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
                    .collect();
                let security_groups = self
                    .get_security_groups(&openshift_instances)
                    .await
                    .unwrap();
                for instance in openshift_instances.iter() {
                    let mut awsi = AWSInstance {
                        instance: instance.clone(),
                        security_groups: vec![],
                    };
                    for sg in security_groups.iter() {
                        let group_identifiers: Vec<&String> = instance
                            .security_groups()
                            .iter()
                            .map(|gi| gi.group_id.as_ref().unwrap())
                            .collect();
                        if group_identifiers.contains(&sg.group_id.as_ref().unwrap()) {
                            awsi.security_groups.push(sg.clone());
                        }
                    }
                }
            }
            Err(err) => return Err(Box::new(err)),
        }
        Ok(openshift_instances)
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

pub struct NetworkInterfaceGatherer<'a> {
    pub client: &'a Client,
    pub loadbalancers: &'a Vec<AWSLoadBalancer>,
}

#[async_trait]
impl<'a> Gatherer for NetworkInterfaceGatherer<'a> {
    type Resource = NetworkInterface;
    async fn gather(&self) -> Result<Vec<Self::Resource>, Box<dyn Error>> {
        debug!("Retrieving ENIs for LoadBalancers");
        let network_interfaces;
        // aws ec2 describe-network-interfaces --filters Name=description,Values="ELB $MC_LB_NAME" --query 'NetworkInterfaces[].PrivateIpAddresses[].PrivateIpAddress' --no-cli-pager --output yaml >> "$TMP_FILE"
        let descriptions: Vec<String> = self
            .loadbalancers
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
        let result = self
            .client
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
            Err(err) => return Err(Box::new(err)),
        }
        Ok(network_interfaces.unwrap())
    }
}
