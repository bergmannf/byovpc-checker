//! This checker provides networking setup checks.
//! It can check the following conditions right now:
//!
//! - Number of subnets in the VPC matches expectation (2 subnets per AZ)
//! - The subnets in the VPC have the expected tags.

use crate::types::{VerificationResult, Verifier};
use log::{debug, info};

use std::collections::{HashMap, HashSet};

pub const PRIVATE_ELB_TAG: &str = "kubernetes.io/role/internal-elb";
pub const PUBLIC_ELB_TAG: &str = "kubernetes.io/role/elb";
pub const CLUSTER_TAG: &str = "kubernetes.io/cluster/";

pub struct ClusterNetwork<'a> {
    clusterid: &'a str,
    configured_subnets: Vec<aws_sdk_ec2::types::Subnet>,
    all_subnets: Vec<aws_sdk_ec2::types::Subnet>,
    routetables: Vec<aws_sdk_ec2::types::RouteTable>,
    subnet_routetable_mapping: HashMap<String, aws_sdk_ec2::types::RouteTable>,
    load_balancers: Vec<aws_sdk_elasticloadbalancingv2::types::LoadBalancer>,
    load_balancer_enis: Vec<aws_sdk_ec2::types::NetworkInterface>,
}

impl<'a> ClusterNetwork<'a> {
    pub fn new(
        clusterid: &'a str,
        configured_subnets: Vec<aws_sdk_ec2::types::Subnet>,
        all_subnets: Vec<aws_sdk_ec2::types::Subnet>,
        routetables: Vec<aws_sdk_ec2::types::RouteTable>,
        load_balancers: Vec<aws_sdk_elasticloadbalancingv2::types::LoadBalancer>,
        load_balancer_enis: Vec<aws_sdk_ec2::types::NetworkInterface>,
    ) -> ClusterNetwork<'a> {
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
            clusterid,
            configured_subnets,
            all_subnets,
            routetables,
            subnet_routetable_mapping: subnet_to_routetables,
            load_balancers,
            load_balancer_enis,
        }
    }

    fn get_public_subnets(&self) -> Vec<String> {
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
            VerificationResult::SubnetTooManyPerAZ(problematic_azs)
        }
    }

    pub fn verify_subnet_tags(&self) -> Vec<VerificationResult> {
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
                if let (Some(key), Some(value)) = (&tag.key, &tag.value) {
                    if key.contains(&CLUSTER_TAG) {
                        missing_cluster_tag = false;
                        if !key.contains(&self.clusterid) && value == "owned" {
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
                verification_results.push(VerificationResult::SubnetMissingClusterTag(
                    subnet_id.clone(),
                ));
            }
            if has_incorrect_cluster_tag {
                verification_results.push(VerificationResult::SubnetIncorrectClusterTag(
                    subnet_id.clone(),
                    incorrect_cluster_tag,
                ));
            }
            if missing_private_elb_tag {
                verification_results.push(VerificationResult::SubnetMissingPrivateElbTag(
                    subnet_id.clone(),
                ));
            }
            if missing_public_elb_tag {
                verification_results.push(VerificationResult::SubnetMissingPublicElbTag(
                    subnet_id.clone(),
                ));
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

    /// Verifies that a LB is using the subnets that are actually configured for the cluster.
    /// This can be incorrect, if subnet tagging was done incorrectly:
    /// See https://access.redhat.com/documentation/en-us/red_hat_openshift_service_on_aws/4/html-single/networking/index#aws-installing-an-aws-load-balancer-operator_aws-load-balancer-operator
    pub fn verify_loadbalancer_subnets(&self) -> Vec<VerificationResult> {
        let mut verification_results = vec![];
        let configured_subnet_ids: HashSet<&str> = self
            .configured_subnets
            .iter()
            .map(|s| s.subnet_id().unwrap())
            .collect();
        for lb in self.load_balancers.iter() {
            for az in lb.availability_zones() {
                let sid = az.subnet_id().unwrap();
                if !configured_subnet_ids.contains(sid) {
                    verification_results.push(VerificationResult::LoadBalancerIncorrectSubnet(
                        lb.load_balancer_arn.as_ref().unwrap().clone(),
                        az.zone_name.as_ref().unwrap().to_string(),
                        sid.to_string(),
                    ))
                }
            }
        }
        if verification_results.len() == 0 {
            verification_results.push(VerificationResult::Success(
                "LoadBalancer subnet associations seem correct".to_string(),
            ));
        }
        verification_results
    }
}

impl<'a> Verifier for ClusterNetwork<'a> {
    fn verify(&self) -> Vec<VerificationResult> {
        let number_result = self.verify_number_of_subnets();
        let lb_result = self.verify_loadbalancer_subnets();
        let mut tag_results = self.verify_subnet_tags();
        tag_results.push(number_result);
        tag_results.extend(lb_result);
        tag_results
    }
}

#[cfg(test)]
mod tests {
    use crate::aws::CLUSTER_TAG_PREFIX;

    use super::*;

    fn make_subnet(
        subnet_id: &str,
        az: &str,
        tags: &HashMap<&str, &str>,
    ) -> aws_sdk_ec2::types::Subnet {
        let tags = tags
            .iter()
            .map(|(k, v)| {
                aws_sdk_ec2::types::Tag::builder()
                    .key(k.to_string())
                    .value(v.to_string())
                    .build()
            })
            .collect();
        aws_sdk_ec2::types::Subnet::builder()
            .subnet_id(subnet_id)
            .availability_zone(az)
            .set_tags(Some(tags))
            .build()
    }

    fn make_private_subnet(
        subnet_id: &str,
        az: &str,
        tags: &HashMap<&str, &str>,
    ) -> (aws_sdk_ec2::types::Subnet, aws_sdk_ec2::types::RouteTable) {
        let private_subnet = make_subnet(subnet_id, az, tags);
        let private_rtb = aws_sdk_ec2::types::RouteTable::builder()
            .associations(
                aws_sdk_ec2::types::RouteTableAssociation::builder()
                    .subnet_id(subnet_id)
                    .build(),
            )
            .build();
        (private_subnet, private_rtb)
    }

    fn make_public_subnet(
        subnet_id: &str,
        az: &str,
        tags: &HashMap<&str, &str>,
    ) -> (aws_sdk_ec2::types::Subnet, aws_sdk_ec2::types::RouteTable) {
        let public_subnet = make_subnet(subnet_id, az, tags);
        let public_rtb = aws_sdk_ec2::types::RouteTable::builder()
            .associations(
                aws_sdk_ec2::types::RouteTableAssociation::builder()
                    .subnet_id(subnet_id)
                    .build(),
            )
            .routes(
                aws_sdk_ec2::types::Route::builder()
                    .destination_cidr_block("0.0.0.0/0")
                    .set_gateway_id(Some("1".to_string()))
                    .build(),
            )
            .build();
        (public_subnet, public_rtb)
    }

    #[test]
    fn test_verify_number_of_subnets_success() {
        let subnet = aws_sdk_ec2::types::Subnet::builder()
            .availability_zone("us-east-1a")
            .build();
        let cn = ClusterNetwork::new(
            "1",
            vec![subnet.clone()],
            vec![subnet.clone()],
            vec![],
            vec![],
            vec![],
        );
        let result = cn.verify_number_of_subnets();
        assert_eq!(
            result,
            VerificationResult::Success("All AZs have the expected number of subnets".to_string())
        )
    }

    #[test]
    fn test_verify_number_of_subnets_fail() {
        let mut subnets = vec![];
        for _ in 1..=3 {
            subnets.push(
                aws_sdk_ec2::types::Subnet::builder()
                    .availability_zone("us-east-1a")
                    .build(),
            );
        }
        let cn = ClusterNetwork::new(
            "1",
            subnets.clone(),
            subnets.clone(),
            vec![],
            vec![],
            vec![],
        );
        let result = cn.verify_number_of_subnets();
        assert_eq!(
            result,
            VerificationResult::SubnetTooManyPerAZ(vec![("us-east-1a".to_string(), 3)])
        )
    }

    #[test]
    fn test_verify_tags_missing_cluster_tag() {
        let clusterid = "1";
        let (public_subnet, public_rtb) =
            make_public_subnet("1", "us-east-1a", &HashMap::from([(PUBLIC_ELB_TAG, "1")]));
        let cn = ClusterNetwork::new(
            clusterid,
            vec![public_subnet.clone()],
            vec![public_subnet.clone()],
            vec![public_rtb.clone()],
            vec![],
            vec![],
        );
        let results = cn.verify_subnet_tags();
        assert_eq!(
            results[0],
            VerificationResult::SubnetMissingClusterTag("1".to_string())
        )
    }

    #[test]
    fn test_verify_tags_incorrect_cluster_tag() {
        let clusterid = "1";
        let (public_subnet, public_rtb) = make_public_subnet(
            "1",
            "us-east-1a",
            &HashMap::from([
                (PUBLIC_ELB_TAG, "1"),
                (&format!("{}{}", CLUSTER_TAG_PREFIX, "2"), "owned"),
            ]),
        );
        let cn = ClusterNetwork::new(
            clusterid,
            vec![public_subnet.clone()],
            vec![public_subnet.clone()],
            vec![public_rtb.clone()],
            vec![],
            vec![],
        );
        let results = cn.verify_subnet_tags();
        assert_eq!(
            results[0],
            VerificationResult::SubnetIncorrectClusterTag(
                public_subnet.subnet_id.unwrap(),
                "kubernetes.io/cluster/2".to_string()
            )
        )
    }
}
