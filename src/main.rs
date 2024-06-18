//! This program provides a quick way to check the setup of AWS and detect
//! possible problems when attempting to run Openshift clusters. It focuses on
//! bring-your-own-VPC checks - meaning the networking setup was performed by
//! the user, not the installer.

mod aws;
mod checks;
mod types;

use checks::network::ClusterNetwork;
use log::{debug, info};
use std::process::exit;
use types::MinimalClusterInfo;

use aws_sdk_ec2::{Client as EC2Client, Error};
use aws_sdk_elasticloadbalancing::Client as ELBv1Client;
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use clap::Parser;

use crate::types::Verifier;

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
            "This check only works for BYOVPC AWS clusters, not: {}",
            cluster_info.cloud_provider
        );
        exit(1)
    }

    let aws_config = crate::aws::aws_setup().await;

    let ec2_client = EC2Client::new(&aws_config);
    let elbv2_client = ELBv2Client::new(&aws_config);
    let elbv1_client = ELBv1Client::new(&aws_config);

    info!("Fetching LoadBalancer data");
    let h1 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            info!("Fetching load balancers");
            let lbs = crate::aws::get_load_balancers(&elbv2_client, &cluster_info)
                .await
                .expect("could not retrieve load balancers");
            let classic_lbs = crate::aws::get_classic_load_balancers(&elbv1_client, &cluster_info)
                .await
                .expect("could not retrieve classic load balancers");
            let ec2_client = ec2_client.clone();
            let lbs = lbs.clone();
            let eni_lbs = crate::aws::get_load_balancer_enis(&ec2_client, &lbs)
                .await
                .expect("could not retrieve ENIs");
            (lbs, classic_lbs, eni_lbs)
        }
    });

    info!("Fetching Subnet data");
    let h2 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            let aws_subnets = crate::aws::get_subnets(&ec2_client, &cluster_info)
                .await
                .expect("Could not retrieve configured subnets");
            let all_subnets = crate::aws::get_all_subnets(&ec2_client, &cluster_info, &aws_subnets)
                .await
                .expect("did not get subnets from vpc");
            let subnet_ids = all_subnets
                .iter()
                .map(|s| s.subnet_id.as_ref().unwrap().clone())
                .collect();
            info!("Fetching all routetables");
            let routetables = crate::aws::get_route_tables(&ec2_client, &subnet_ids)
                .await
                .expect("Could not retrieve routetables");
            (aws_subnets, all_subnets, routetables)
        }
    });

    info!("Fetching instances and security groups");
    let h3 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            let instances = crate::aws::get_instances(&ec2_client, &cluster_info)
                .await
                .expect("Could not retrieve instances");
            let security_groups = crate::aws::get_security_groups(&ec2_client, &cluster_info)
                .await
                .expect("Could not retrieve security group");
            (instances, security_groups)
        }
    });

    let (lbs, classic_lbs, lb_enis) = h1.await.unwrap();
    let (configured_subnets, all_subnets, routetables) = h2.await.unwrap();
    let (instances, security_groups) = h3.await.unwrap();

    debug!("{:?}", instances);
    debug!("{:?}", security_groups);

    let cn = ClusterNetwork::new(
        &options.clusterid,
        &cluster_info.cluster_infra_name,
        configured_subnets,
        all_subnets,
        routetables,
        lbs,
        lb_enis,
        classic_lbs,
    );
    for res in cn.verify() {
        println!("{}", res);
    }
    Ok(())
}
