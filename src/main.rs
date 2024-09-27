//! This program provides a quick way to check the setup of AWS and detect
//! possible problems when attempting to run Openshift clusters. It focuses on
//! bring-your-own-VPC checks - meaning the networking setup was performed by
//! the user, not the installer.

mod checks;
mod gatherer;
mod types;

use aws_sdk_ec2::Error;
use checks::{dns::HostedZoneChecksBuilder, network::ClusterNetworkBuilder};
use clap::Parser;
use colored::Colorize;
use gatherer::aws::AWSClusterData;
use std::process::exit;
use types::MinimalClusterInfo;

use crate::types::Verifier;

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Checks,
    Debug,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum Check {
    All,
    Network,
    HostedZone,
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
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Checks)]
    format: OutputFormat,
    #[arg(long, value_enum, default_values_t = vec![Check::All])]
    checks: Vec<Check>,
}

fn setup_checks(
    options: Options,
    cluster_info: &MinimalClusterInfo,
    aws_data: AWSClusterData,
) -> Vec<Box<dyn Verifier + '_>> {
    let mut checks: Vec<Box<dyn Verifier>> = vec![];
    for c in options.checks {
        match c {
            Check::All => {
                let mut cnb = ClusterNetworkBuilder::default();
                let cn = cnb
                    .cluster_info(&cluster_info)
                    .all_subnets(aws_data.subnets.clone())
                    .routetables(aws_data.routetables.clone())
                    .load_balancers(aws_data.load_balancers.clone())
                    .load_balancer_enis(aws_data.load_balancer_enis.clone())
                    .build()
                    .unwrap();
                checks.push(Box::new(cn));
                let mut hzb = HostedZoneChecksBuilder::default();
                let hz = hzb
                    .hosted_zones(aws_data.hosted_zones.clone())
                    .load_balancers(aws_data.load_balancers.clone())
                    .build()
                    .unwrap();
                checks.push(Box::new(hz));
            }
            Check::Network => {
                let mut cnb = ClusterNetworkBuilder::default();
                let cn = cnb
                    .cluster_info(&cluster_info)
                    .all_subnets(aws_data.subnets.clone())
                    .routetables(aws_data.routetables.clone())
                    .load_balancers(aws_data.load_balancers.clone())
                    .load_balancer_enis(aws_data.load_balancer_enis.clone())
                    .build()
                    .unwrap();
                checks.push(Box::new(cn));
            }
            Check::HostedZone => {
                let mut hzb = HostedZoneChecksBuilder::default();
                let hz = hzb
                    .hosted_zones(aws_data.hosted_zones.clone())
                    .load_balancers(aws_data.load_balancers.clone())
                    .build()
                    .unwrap();
                checks.push(Box::new(hz));
            }
        }
    }
    checks
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

    match options.format {
        OutputFormat::Debug => {
            println!("{}", &format!("{:#?}", aws_data))
        }
        OutputFormat::Checks => {
            let checks = setup_checks(options, &cluster_info, aws_data);
            for check in checks {
                for res in check.verify() {
                    println!("{}", res);
                }
            }
        }
    }
    Ok(())
}
