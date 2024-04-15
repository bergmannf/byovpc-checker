//! Shared types that are used throughout the application.

use colored::Colorize;
use log::debug;
use std::{error::Error, fmt::Display, process::Command};

/// Indicates an expected property did not hold - should indicate a failure.
#[derive(Debug)]
pub struct InvariantError {
    pub msg: String,
}

impl Display for InvariantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl Error for InvariantError {
    fn description(&self) -> &str {
        &self.msg
    }
}

/// Trait to wrap running the checks to be performed. Every check should return
/// a number of VerificationResults that can be printed.
pub trait Verifier {
    fn verify(&self) -> Vec<VerificationResult>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum ClusterType {
    Osd,
    Rosa,
    Hypershift,
}

pub struct MinimalClusterInfo {
    pub cluster_id: String,
    pub cluster_infra_name: String,
    pub cluster_type: ClusterType,
    pub cloud_provider: String,
    pub subnets: Vec<String>,
}

impl MinimalClusterInfo {
    fn get_cluster_json(clusterid: &String) -> serde_json::Value {
        let mut ocm = Command::new("ocm");
        ocm.arg("describe")
            .arg("cluster")
            .arg("--json")
            .arg(clusterid);

        let stdout = ocm.output().expect("could not get output").stdout;
        let stdout_str = std::str::from_utf8(&stdout).expect("retrieving stdout failed");
        debug!("OCM Cluster information: {:}", stdout_str);
        serde_json::from_str(stdout_str).expect("failed to parse json")
    }

    pub fn get_cluster_info(clusterid: &String) -> Self {
        let cluster_json = MinimalClusterInfo::get_cluster_json(clusterid);
        let x = cluster_json["aws"]["subnet_ids"]
            .as_array()
            .expect("Subnets must be an array - is this not a BYOVPC cluster?");
        let subnets: Vec<String> = x
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("converting subnet to str failed")
                    .to_string()
            })
            .collect();
        let cluster_type = MinimalClusterInfo::cluster_type(&cluster_json).expect(
            "Could not determine product - only OSD (on AWS), Rosa and Hypershift are supported.",
        );
        debug!("Product is: {:?}", cluster_type);
        let cluster_infra_name = match cluster_type {
            ClusterType::Hypershift => cluster_json.get("id").unwrap().as_str().unwrap(),
            _ => cluster_json
                .get("infra_id")
                .expect("did not find a infra id for the cluster")
                .as_str()
                .unwrap(),
        };
        MinimalClusterInfo {
            cluster_id: clusterid.to_string(),
            cluster_infra_name: cluster_infra_name.to_string(),
            cluster_type,
            cloud_provider: cluster_json["cloud_provider"]["id"]
                .as_str()
                .unwrap()
                .to_string(),
            subnets,
        }
    }

    fn cluster_type(cluster_json: &serde_json::Value) -> Option<ClusterType> {
        debug!("Checking cluster type");
        if let Some(hypershift) = cluster_json
            .get("hypershift")
            .and_then(|v| v.get("enabled"))
        {
            debug!("Checking hypershift: {}", hypershift);
            if hypershift == true {
                return Some(ClusterType::Hypershift);
            }
        }
        if let Some(product) = cluster_json.get("product").and_then(|v| v.get("id")) {
            debug!("Checking OSD|Rosa: {}", product);
            if product == "OSD" {
                return Some(ClusterType::Osd);
            } else if product == "rosa" {
                return Some(ClusterType::Rosa);
            }
        }
        return None;
    }
}

/// VerificationResult list all error conditions that can occur. These should be
/// detailed enough to allow the user to fix the problem.
#[derive(Debug, PartialEq, Eq)]
pub enum VerificationResult {
    Success(String),
    SubnetTooManyPerAZ(Vec<(String, u8)>),
    SubnetMissingClusterTag(String),
    SubnetIncorrectClusterTag(String, String),
    SubnetMissingPrivateElbTag(String),
    SubnetMissingPublicElbTag(String),
    LoadBalancerIncorrectSubnet(String, String, String),
}

impl Display for VerificationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationResult::Success(msg) => f.write_str(&msg.green().to_string()),
            VerificationResult::SubnetTooManyPerAZ(azs) => {
                let results = azs.iter().map(|a| {
                    let msg = format!(
                        "AZ {} has {}: {}",
                        a.0.blue(),
                        "too many subnets".red(),
                        a.1
                    );
                    f.write_str(&msg)
                });
                results.collect()
            }
            VerificationResult::SubnetMissingClusterTag(subnet) => f.write_str(&format!(
                "Subnet {} is {}",
                subnet.blue(),
                "missing a cluster tag".red()
            )),
            VerificationResult::SubnetIncorrectClusterTag(subnet, tag) => f.write_str(&format!(
                "Subnet {} has a non-shared cluster tag of a different cluster: {}",
                subnet.blue(),
                tag.red()
            )),
            VerificationResult::SubnetMissingPrivateElbTag(subnet) => f.write_str(&format!(
                "Subnet {} is {}: {}",
                subnet.blue(),
                "missing private-elb tag".red(),
                crate::checks::network::PRIVATE_ELB_TAG.red()
            )),
            VerificationResult::SubnetMissingPublicElbTag(subnet) => f.write_str(&format!(
                "Subnet {} is {}: {}",
                subnet.blue(),
                "missing public-elb tag".red(),
                crate::checks::network::PUBLIC_ELB_TAG.red()
            )),
            VerificationResult::LoadBalancerIncorrectSubnet(lb, az, subnet) => {
                f.write_str(&format!(
                    "LoadBalancer {} is {} in AZ {}",
                    lb.blue(),
                    format!(
                        "using a subnet ({}) not configured for this cluster",
                        subnet
                    )
                    .red(),
                    az.blue()
                ))
            }
        }
    }
}
