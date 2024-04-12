mod aws;
mod types;
mod vpc;

use types::MinimalClusterInfo;
use vpc::ClusterNetwork;
use std::process::exit;

use aws_sdk_ec2::{Client as EC2Client, Error};
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use clap::Parser;

use crate::types::Verifier;

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

    let aws_config = crate::aws::aws_setup().await;
    let ec2_client = EC2Client::new(&aws_config);
    let elbv2_client = ELBv2Client::new(&aws_config);
    let lbs = crate::aws::get_load_balancers(&elbv2_client, &cluster_info).await.expect("could not retrieve load balancers");
    println!("Found load-balancers for cluster: {:?}", lbs);
    let aws_subnets = crate::aws::get_subnets(&ec2_client, &cluster_info.subnets).await;
    let configured_subnets = aws_subnets.as_ref().unwrap().subnets.as_ref().unwrap().clone();
    let all_subnets = crate::aws::get_all_subnets(&ec2_client, &aws_subnets.unwrap().subnets.unwrap()).await.expect("did not get subnets from vpc");
    let subnet_ids = all_subnets.iter().map(|s| s.subnet_id.as_ref().unwrap().clone()).collect();
    let aws_routetables = crate::aws::get_route_tables(&ec2_client, &subnet_ids).await;

    let css = ClusterNetwork::new(&options.clusterid,
                                  configured_subnets,
                                  all_subnets,
                                  aws_routetables.unwrap().route_tables.unwrap().to_vec());
    for res in css.verify() {
        println!("{}", res);
    }
    Ok(())
}
