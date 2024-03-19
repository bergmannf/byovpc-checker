use colored::Colorize;
use log::debug;
use std::{error::Error, fmt::Display, process::Command};

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

pub struct MinimalClusterInfo {
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
        MinimalClusterInfo {
            cloud_provider: cluster_json["cloud_provider"]["id"]
                .as_str()
                .unwrap()
                .to_string(),
            subnets,
        }
    }
}

pub enum VerificationResult {
    Success(String),
    TooManySubnetsPerAZ(Vec<(String, u8)>),
    MissingClusterTag(String),
    IncorrectClusterTag(String, String),
    MissingPrivateElbTag(String),
    MissingPublicElbTag(String),
}

impl Display for VerificationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationResult::Success(msg) => f.write_str(&msg.green().to_string()),
            VerificationResult::TooManySubnetsPerAZ(azs) => {
                let results = azs.iter().map(|a| {
                    let msg = format!("Subnet {} has too many subnets: {}", a.0, a.1).red();
                    f.write_str(&msg)
                });
                results.collect()
            }
            VerificationResult::MissingClusterTag(subnet) => f.write_str(&format!(
                "Subnet {} is {}",
                subnet.red(),
                "missing a cluster tag".red()
            )),
            VerificationResult::IncorrectClusterTag(subnet, tag) => f.write_str(&format!(
                "Subnet {} has a non-shared cluster tag of a different cluster: {}",
                subnet.red(),
                tag.red()
            )),
            VerificationResult::MissingPrivateElbTag(subnet) => f.write_str(&format!(
                "Subnet {} is missing private-elb tag: {}",
                subnet.red(),
                crate::vpc::PRIVATE_ELB_TAG.red()
            )),
            VerificationResult::MissingPublicElbTag(subnet) => f.write_str(&format!(
                "Subnet {} is missing public-elb tag: {}",
                subnet.red(),
                crate::vpc::PUBLIC_ELB_TAG.red()
            )),
        }
    }
}
