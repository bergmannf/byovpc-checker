use crate::types::VerificationResult;
use log::{debug, info};

use std::collections::HashMap;

pub const PRIVATE_ELB_TAG: &str = "kubernetes.io/role/internal-elb";
pub const PUBLIC_ELB_TAG: &str = "kubernetes.io/role/elb";
pub const CLUSTER_TAG: &str = "kubernetes.io/cluster/";

pub struct ClusterNetwork {
    configured_subnets: Vec<aws_sdk_ec2::types::Subnet>,
    all_subnets: Vec<aws_sdk_ec2::types::Subnet>,
    routetables: Vec<aws_sdk_ec2::types::RouteTable>,
    subnet_routetable_mapping: HashMap<String, aws_sdk_ec2::types::RouteTable>,
}

impl ClusterNetwork {
    pub fn new(
        configured_subnets: Vec<aws_sdk_ec2::types::Subnet>,
        all_subnets: Vec<aws_sdk_ec2::types::Subnet>,
        routetables: Vec<aws_sdk_ec2::types::RouteTable>,
    ) -> ClusterNetwork {
        let mut subnet_to_routetables: HashMap<String, aws_sdk_ec2::types::RouteTable> =
            HashMap::new();
        for subnet in all_subnets.iter() {
            let rtb: Vec<&aws_sdk_ec2::types::RouteTable> = routetables
                .iter()
                .filter(|rtb| {
                    rtb.associations
                        .iter()
                        .any(|a| a.iter().any(|b| b.subnet_id() == subnet.subnet_id()))
                })
                .collect();
            if let Some(rt) = rtb.first() {
                let drt = (**rt).clone();
                subnet_to_routetables.insert(subnet.subnet_id.clone().unwrap(), drt);
            }
        }
        ClusterNetwork {
            configured_subnets,
            all_subnets,
            routetables,
            subnet_routetable_mapping: subnet_to_routetables,
        }
    }

    pub fn get_public_subnets(&self) -> Vec<String> {
        let mut public_subnets = Vec::new();
        for (subnet, rtb) in self.subnet_routetable_mapping.iter() {
            let routes = rtb.routes.as_ref().map(|r| r);
            if let Some(rs) = routes {
                for r in rs {
                    let is_0_cidr = r
                        .destination_cidr_block
                        .clone()
                        .is_some_and(|f| f == "0.0.0.0/0");
                    if is_0_cidr && (r.transit_gateway_id.is_some() || r.gateway_id.is_some()) {
                        public_subnets.push(subnet.clone())
                    }
                }
            }
        }
        return public_subnets;
    }

    fn get_private_subnets(&self) -> Vec<String> {
        let mut private_subnets = Vec::new();
        for (subnet, rtb) in self.subnet_routetable_mapping.iter() {
            let routes = rtb.routes.as_ref().map(|r| r);
            if let Some(rs) = routes {
                let has_0_cidr = rs.iter().any(|r| {
                    r.destination_cidr_block
                        .clone()
                        .is_some_and(|f| f == "0.0.0.0/0")
                });
                if !has_0_cidr {
                    private_subnets.push(subnet.clone());
                    break;
                }
                for r in rs {
                    let is_0_cidr = r
                        .destination_cidr_block
                        .clone()
                        .is_some_and(|f| f == "0.0.0.0/0");
                    if is_0_cidr && (r.nat_gateway_id.is_some()) {
                        private_subnets.push(subnet.clone());
                    }
                }
            }
        }
        return private_subnets;
    }

    pub fn verify_number_of_subnets(&self) -> VerificationResult {
        info!("Checking number of subnets per AZ");
        let mut subnets_per_az: HashMap<String, u8> = HashMap::new();
        let mut problematic_azs: Vec<(String, u8)> = Vec::new();
        for subnet in self.all_subnets.iter() {
            let az = subnet.availability_zone.clone().unwrap();
            *subnets_per_az.entry(az).or_insert(0) += 1;
        }
        for (az, number) in subnets_per_az {
            if number > 2 {
                problematic_azs.push((az, number));
            }
        }
        if problematic_azs.len() == 0 {
            VerificationResult::Success("All AZs have the expected number of subnets".to_string())
        } else {
            VerificationResult::TooManySubnetsPerAZ(problematic_azs)
        }
    }

    pub fn verify_subnet_tags(&self, clusterid: &String) -> Vec<VerificationResult> {
        info!("Checking tags per subnet");
        let mut verification_results = Vec::new();
        for subnet in self.all_subnets.iter() {
            let mut missing_cluster_tag = true;
            let mut incorrect_cluster_tag = String::new();
            let mut missing_private_elb_tag = true;
            let mut missing_public_elb_tag = true;
            let subnet_id = subnet.subnet_id().unwrap().to_string();
            let tags = subnet.tags();
            debug!("Checking subnet: {}", subnet_id);
            for tag in tags {
                if let (Some(key), Some(value)) = (tag.key.clone(), tag.value.clone()) {
                    if key.contains(&CLUSTER_TAG) {
                        missing_cluster_tag = false;
                        if !key.contains(clusterid) && value == "owned" {
                            incorrect_cluster_tag = key.clone();
                        }
                    }
                    if !self.get_private_subnets().contains(&subnet_id) {
                        missing_private_elb_tag = false;
                    }
                    if !self.get_public_subnets().contains(&subnet_id) {
                        missing_public_elb_tag = false;
                    }
                    if self.get_private_subnets().contains(&subnet_id)
                        && key.contains(&PRIVATE_ELB_TAG)
                    {
                        missing_private_elb_tag = false;
                    }
                    if self.get_public_subnets().contains(&subnet_id)
                        && key.contains(&PUBLIC_ELB_TAG)
                    {
                        missing_public_elb_tag = false;
                    }
                }
            }
            let has_incorrect_cluster_tag = incorrect_cluster_tag.len() > 0;
            if missing_cluster_tag {
                verification_results.push(VerificationResult::MissingClusterTag(subnet_id.clone()));
            }
            if has_incorrect_cluster_tag {
                verification_results.push(VerificationResult::IncorrectClusterTag(
                    subnet_id.clone(),
                    incorrect_cluster_tag,
                ));
            }
            if missing_private_elb_tag {
                verification_results
                    .push(VerificationResult::MissingPrivateElbTag(subnet_id.clone()));
            }
            if missing_public_elb_tag {
                verification_results
                    .push(VerificationResult::MissingPublicElbTag(subnet_id.clone()));
            }
            if !missing_cluster_tag
                && !has_incorrect_cluster_tag
                && !missing_public_elb_tag
                && !missing_private_elb_tag
            {
                verification_results.push(VerificationResult::Success(format!(
                    "Subnet {} seems correctly setup.",
                    subnet_id
                )))
            }
        }
        verification_results
    }
}
