//! Shared types that are used throughout the application.

use colored::Colorize;
use derive_builder::Builder;
use log::{debug, warn};
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClusterType {
    Osd,
    Rosa,
    Hypershift,
}

#[derive(Builder, Clone, Debug)]
pub struct MinimalClusterInfo {
    pub cluster_id: String,
    #[builder(default = "\"\".to_string()")]
    pub cluster_infra_name: String,
    #[builder(default = "ClusterType::Osd")]
    pub cluster_type: ClusterType,
    #[builder(default = "\"AWS\".to_string()")]
    pub cloud_provider: String,
    #[builder(default = "vec![]")]
    pub subnets: Vec<String>,
    #[builder(default = "None")]
    pub base_domain: Option<String>,
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
        let sxs = cluster_json
            .get("aws")
            .and_then(|v| v.get("subnet_ids"))
            .and_then(|v| v.as_array());
        let subnets: Vec<String> = if let Some(sx) = sxs {
            if sx.is_empty() {
                warn!("No subnet ids configured - this will make some checks relying on this useless.");
                vec![]
            } else {
                sx.iter()
                    .map(|v| {
                        v.as_str()
                            .expect("converting subnet to str failed")
                            .to_string()
                    })
                    .collect()
            }
        } else {
            warn!("No subnet ids configured - this will make some checks relying on this useless.");
            vec![]
        };
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
            base_domain: MinimalClusterInfo::base_domain(&cluster_json),
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
            if product == "osd" {
                return Some(ClusterType::Osd);
            } else if product == "rosa" {
                return Some(ClusterType::Rosa);
            }
        }
        return None;
    }

    fn base_domain(cluster_json: &serde_json::Value) -> Option<String> {
        let console_url = cluster_json
            .get("api")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str());
        console_url.map_or_else(
            || None,
            |s| {
                let without_port: Vec<&str> = s.split_terminator(":").collect();
                assert_eq!(without_port.len(), 3);
                let parts: Vec<&str> = without_port[1].split_terminator(".").collect();
                let bd = parts[2..].join(".");
                debug!("Base Domain calculated as: {}", bd);
                // FIXME: Hard coded stripping of parts from the URL is not nice
                Some(bd)
            },
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Info,
    Warning,
    Critical,
}

/// VerificationResult list all error conditions that can occur. These should be
/// detailed enough to allow the user to fix the problem.
#[derive(Debug, PartialEq, Eq)]
pub struct VerificationResult {
    pub message: String,
    pub severity: Severity,
}

impl Display for VerificationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.severity {
            Severity::Ok => f.write_str(&(format!("{} {}", "Ⓞ -".green(), self.message.green()))),
            Severity::Info => f.write_str(&format!("{} {}", "Ⓘ -".blue(), self.message.blue())),
            Severity::Warning => {
                f.write_str(&format!("{} {}", "Ⓦ -".yellow(), self.message.yellow()))
            }
            Severity::Critical => f.write_str(&format!("{} {}", "Ⓔ -".red(), self.message.red())),
        }
    }
}
