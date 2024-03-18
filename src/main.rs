use std::collections::HashMap;
use std::fmt::Display;
use std::process::exit;
use std::process::Command;

use aws_config::BehaviorVersion;
use aws_config::SdkConfig;
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_ec2::error::SdkError;
use aws_sdk_ec2::operation::describe_route_tables::DescribeRouteTablesError;
use aws_sdk_ec2::operation::describe_route_tables::DescribeRouteTablesOutput;
use aws_sdk_ec2::operation::describe_subnets::DescribeSubnetsError;
use aws_sdk_ec2::operation::describe_subnets::DescribeSubnetsOutput;
use aws_sdk_ec2::types::Filter;
use aws_sdk_ec2::{Client, Error};
use clap::Parser;
use colored::Colorize;

use log::{debug, info};

const PRIVATE_ELB_TAG: &str = "kubernetes.io/role/internal-elb";
const PUBLIC_ELB_TAG: &str = "kubernetes.io/role/elb";
const CLUSTER_TAG: &str = "kubernetes.io/cluster/";

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about,
    long_about = "Verifies if the VPC setup for the cluster is valid"
)]
struct Options {
    #[arg(short, long)]
    clusterid: String,
    #[command(flatten)]
    verbose: clap_verbosity_flag::Verbosity,
}

struct MinimalClusterInfo {
    cloud_provider: String,
    subnets: Vec<String>,
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

    fn get_cluster_info(clusterid: &String) -> Self {
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
            cloud_provider: cluster_json["cloud_provider"]["id"].as_str().unwrap().to_string(),
            subnets,
        }
    }
}

enum VerificationResult {
    Success(String),
    TooManySubnetsPerAZ(Vec<(String, u8)>),
    MissingClusterTag(String),
    IncorrectClusterTag(String, String),
    MissingPrivateElbTag(String),
    MissingPublicElbTag(String)
}

impl Display for VerificationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationResult::Success(msg) => {
                f.write_str(&msg.green().to_string())
            }
            VerificationResult::TooManySubnetsPerAZ(azs) => {
                let results = azs.iter().map(|a| {
                    let msg = format!("Subnet {} has too many subnets: {}", a.0, a.1).red();
                    f.write_str(&msg)
                });
                results.collect()
            }
            VerificationResult::MissingClusterTag(subnet) => {
                f.write_str(&format!("Subnet {} is {}", subnet.red(), "missing a cluster tag".red()))
            }
            VerificationResult::IncorrectClusterTag(subnet,tag ) => {
                f.write_str(&format!("Subnet {} has a non-shared cluster tag of a different cluster: {}", subnet.red(), tag.red()))
            }
            VerificationResult::MissingPrivateElbTag(subnet) => {
                f.write_str(&format!("Subnet {} is missing private-elb tag: {}", subnet.red(), PRIVATE_ELB_TAG.red()))
            }
            VerificationResult::MissingPublicElbTag(subnet) => {
                f.write_str(&format!("Subnet {} is missing public-elb tag: {}", subnet.red(), PUBLIC_ELB_TAG.red()))
            }
        }
    }
}

struct ClusterNetwork {
    vpc: String,
    configured_subnets: Vec<aws_sdk_ec2::types::Subnet>,
    all_subnets: Vec<aws_sdk_ec2::types::Subnet>,
    routetables: Vec<aws_sdk_ec2::types::RouteTable>,
    subnet_routetable_mapping: HashMap<String, aws_sdk_ec2::types::RouteTable>,
}

impl ClusterNetwork {
    pub fn new(vpc: String,
               configured_subnets: Vec<aws_sdk_ec2::types::Subnet>,
               all_subnets: Vec<aws_sdk_ec2::types::Subnet>,
               routetables: Vec<aws_sdk_ec2::types::RouteTable>) -> ClusterNetwork {
        let mut subnet_to_routetables: HashMap<String, aws_sdk_ec2::types::RouteTable> = HashMap::new();
        for subnet in all_subnets.iter() {
            let rtb: Vec<&aws_sdk_ec2::types::RouteTable> = routetables.iter()
                .filter(|rtb| rtb.associations.iter()
                       .any(|a| a.iter().
                            any(|b| b.subnet_id() == subnet.subnet_id())))
                .collect();
            if let Some(rt) = rtb.first() {
                let drt = (**rt).clone();
                subnet_to_routetables.insert(subnet.subnet_id.clone().unwrap(),
                                             drt);
            }
        }
        ClusterNetwork { vpc, configured_subnets, all_subnets, routetables, subnet_routetable_mapping: subnet_to_routetables }
    }

    pub fn get_public_subnets(&self) -> Vec<String> {
        let mut public_subnets = Vec::new();
        for (subnet, rtb) in self.subnet_routetable_mapping.iter() {
            let routes = rtb.routes.as_ref().map(|r| r);
            if let Some(rs) = routes {
                for r in rs {
                    let is_0_cidr = r.destination_cidr_block.clone().is_some_and(|f| f == "0.0.0.0/0");
                    if is_0_cidr && (r.transit_gateway_id.is_some() || r.gateway_id.is_some()) {
                        public_subnets.push(subnet.clone())
                    }
                }
            }
        }
        return public_subnets
    }

    fn get_private_subnets(&self) -> Vec<String> {
        let mut private_subnets = Vec::new();
        for (subnet, rtb) in self.subnet_routetable_mapping.iter() {
            let routes = rtb.routes.as_ref().map(|r| r);
            if let Some(rs) = routes {
                let has_0_cidr = rs.iter().any(|r| r.destination_cidr_block.clone().is_some_and(|f| f == "0.0.0.0/0"));
                if ! has_0_cidr {
                    private_subnets.push(subnet.clone());
                    break
                }
                for r in rs {
                    let is_0_cidr = r.destination_cidr_block.clone().is_some_and(|f| f == "0.0.0.0/0");
                    if is_0_cidr && (r.nat_gateway_id.is_some()) {
                        private_subnets.push(subnet.clone());
                    }
                }
            }
        }
        return private_subnets
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
                    if self.get_private_subnets().contains(&subnet_id) && key.contains(&PRIVATE_ELB_TAG) {
                        missing_private_elb_tag = false;
                    }
                    if self.get_public_subnets().contains(&subnet_id) && key.contains(&PUBLIC_ELB_TAG) {
                        missing_public_elb_tag = false;
                    }
                }
            }
            let has_incorrect_cluster_tag = incorrect_cluster_tag.len() > 0;
            if missing_cluster_tag {
                verification_results.push(VerificationResult::MissingClusterTag(subnet_id.clone()));
            }
            if has_incorrect_cluster_tag {
                verification_results.push(VerificationResult::IncorrectClusterTag(subnet_id.clone(), incorrect_cluster_tag));
            }
            if missing_private_elb_tag {
                verification_results.push(VerificationResult::MissingPrivateElbTag(subnet_id.clone()));
            }
            if missing_public_elb_tag {
                verification_results.push(VerificationResult::MissingPublicElbTag(subnet_id.clone()));
            }
            if !missing_cluster_tag && !has_incorrect_cluster_tag && !missing_public_elb_tag && !missing_private_elb_tag {
                verification_results.push(VerificationResult::Success(format!("Subnet {} seems correctly setup.", subnet_id)))
            }
        }
        verification_results
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let options = Options::parse();
    env_logger::Builder::new()
        .filter_level(options.verbose.log_level_filter())
        .init();
    if options.clusterid.is_empty() {
        eprintln!("Must set a clusterid to proceed.");
        exit(1);
    }

    let cluster_info = MinimalClusterInfo::get_cluster_info(&options.clusterid);
    if cluster_info.cloud_provider != "aws" {
        eprintln!("This check only works for BYOVPC AWS clusters, not: {}", cluster_info.cloud_provider);
        exit(1)
    }

    let aws_config = aws_setup().await;
    let ec2_client = Client::new(&aws_config);
    let aws_subnets = ec2_client.describe_subnets().set_subnet_ids(Some(cluster_info.subnets.clone())).send().await;
    let aws_unwrapped_subnets = aws_subnets.unwrap().subnets.unwrap();
    let vpc_id = aws_unwrapped_subnets[0].vpc_id().unwrap().to_string();
    let aws_subnets_by_vpc = get_subnets_by_vpc(&ec2_client, vpc_id.as_str()).await;
    let aws_unwrapped_subnets_by_vpc = aws_subnets_by_vpc.unwrap().subnets.unwrap();
    let aws_routetables = get_route_tables(&ec2_client, &cluster_info).await;
    let css = ClusterNetwork::new(vpc_id,
                                  aws_unwrapped_subnets.to_vec(),
                                  aws_unwrapped_subnets_by_vpc.to_vec(),
                                  aws_routetables.unwrap().route_tables.unwrap().to_vec());
    println!("{}", css.verify_number_of_subnets());
    for res in css.verify_subnet_tags(&options.clusterid) {
        println!("{}", res);
    }
    Ok(())
}

async fn aws_setup() -> SdkConfig {
    let region_provider = RegionProviderChain::default_provider().or_else("us-east-1");
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    return config
}

async fn get_subnets_by_vpc(ec2_client: &Client, vpc_id: &str) -> Result<DescribeSubnetsOutput, SdkError<DescribeSubnetsError>> {
    let subnets_filter = Filter::builder().
        name("vpc-id").
        values(vpc_id).
        build();
    return ec2_client.describe_subnets().set_filters(Some(vec![subnets_filter])).send().await;
}

async fn get_route_tables(ec2_client: &Client, cluster_info: &MinimalClusterInfo) -> Result<DescribeRouteTablesOutput, SdkError<DescribeRouteTablesError>> {
    let rtb_filter = Filter::builder().
        name("association.subnet-id").
        set_values(Some(cluster_info.subnets.clone())).
        build();
    return ec2_client.describe_route_tables().set_filters(Some(vec![rtb_filter])).send().await;
}
