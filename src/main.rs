//! This program provides a quick way to check the setup of AWS and detect
//! possible problems when attempting to run Openshift clusters. It focuses on
//! bring-your-own-VPC checks - meaning the networking setup was performed by
//! the user, not the installer.

mod checks;
mod gatherer;
mod types;

use aws_sdk_ec2::Error;
use checks::network::ClusterNetwork;
use clap::Parser;
use std::process::exit;
use types::MinimalClusterInfo;

use crate::types::Verifier;

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum Check {
    All,
    Network,
}

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Verifies if the VPC setup for the cluster is valid. AWS configuration must be setup to access the cluster's AWS account.",
    long_about = "Verifies if the VPC setup for the cluster is valid. AWS configuration must be setup to access the cluster's AWS account."
)]
struct Options {
    #[arg(short, long)]
    clusterid: String,
    #[command(flatten)]
    verbose: clap_verbosity_flag::Verbosity,
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
    #[arg(long, value_enum, default_values_t = vec![Check::All])]
    checks: Vec<Check>,
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
        eprintln!(
            "This check only works for AWS clusters, not: {}",
            cluster_info.cloud_provider
        );
        exit(1)
    }

    let aws_data = crate::gatherer::aws::gather(&cluster_info).await;

    let cn = ClusterNetwork::new(
        &cluster_info,
        aws_data.subnets,
        aws_data.routetables,
        aws_data.load_balancers,
        aws_data.load_balancer_enis,
        aws_data.classic_load_balancers,
    );
    for res in cn.verify() {
        println!("{}", res);
    }
    Ok(())
}
