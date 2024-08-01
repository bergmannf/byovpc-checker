//! This program provides a quick way to check the setup of AWS and detect
//! possible problems when attempting to run Openshift clusters. It focuses on
//! bring-your-own-VPC checks - meaning the networking setup was performed by
//! the user, not the installer.

mod checks;
mod gatherer;
mod types;

use checks::network::ClusterNetwork;
use log::{debug, info};
use std::process::exit;
use types::MinimalClusterInfo;

use aws_sdk_ec2::{Client as EC2Client, Error};
use aws_sdk_elasticloadbalancing::Client as ELBv1Client;
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use clap::Parser;

use crate::gatherer::Gatherer;
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
            "This check only works for AWS clusters, not: {}",
            cluster_info.cloud_provider
        );
        exit(1)
    }

    let aws_config = crate::gatherer::aws::aws_setup().await;

    let ec2_client = EC2Client::new(&aws_config);
    let elbv2_client = ELBv2Client::new(&aws_config);
    let elbv1_client = ELBv1Client::new(&aws_config);

    info!("Fetching LoadBalancer data");
    let h1 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            info!("Fetching load balancers");
            let lbs = crate::gatherer::aws::loadbalancerv2::LoadBalancerGatherer {
                client: &elbv2_client,
                cluster_info: &cluster_info,
            }
            .gather()
            .await
            .expect("could not retrieve load balancers");
            let classic_lbs =
                crate::gatherer::aws::get_classic_load_balancers(&elbv1_client, &cluster_info)
                    .await
                    .expect("could not retrieve classic load balancers");
            let ec2_client = ec2_client.clone();
            let lbs = lbs.clone();
            let mut mlbs: Vec<crate::gatherer::aws::shared_types::AWSLoadBalancer> = lbs
                .clone()
                .into_iter()
                .map(|l| crate::gatherer::aws::shared_types::AWSLoadBalancer::ModernLoadBalancer(l))
                .collect();
            let mut clbs: Vec<crate::gatherer::aws::shared_types::AWSLoadBalancer> = classic_lbs
                .clone()
                .into_iter()
                .map(|l| {
                    crate::gatherer::aws::shared_types::AWSLoadBalancer::ClassicLoadBalancer(l)
                })
                .collect();
            clbs.append(&mut mlbs);
            let enig = crate::gatherer::aws::ec2::NetworkInterfaceGatherer {
                client: &ec2_client,
                loadbalancers: &clbs,
            };
            let eni_lbs = enig.gather().await.expect("could not retrieve ENIs");
            (lbs, classic_lbs, eni_lbs)
        }
    });

    info!("Fetching Subnet data");
    let h2 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            let sg = crate::gatherer::aws::ec2::ConfiguredSubnetGatherer {
                client: &ec2_client,
                cluster_info: &cluster_info,
            };
            let all_subnets = sg
                .gather()
                .await
                .expect("Could not retrieve configured subnets");
            let subnet_ids = all_subnets
                .iter()
                .map(|s| s.subnet_id.as_ref().unwrap().clone())
                .collect();
            info!("Fetching all routetables");
            let rtg = crate::gatherer::aws::ec2::RouteTableGatherer {
                client: &ec2_client,
                subnet_ids: &subnet_ids,
            };
            let routetables = rtg.gather().await.expect("Could not retrieve routetables");
            (all_subnets, routetables)
        }
    });

    info!("Fetching instances and security groups");
    let h3 = tokio::spawn({
        let cluster_info = cluster_info.clone();
        let ec2_client = ec2_client.clone();
        async move {
            let instances = crate::gatherer::aws::ec2::InstanceGatherer {
                client: &ec2_client,
                cluster_info: &cluster_info,
            }
            .gather()
            .await
            .expect("Could not retrieve instances");
            instances
        }
    });

    let (lbs, classic_lbs, lb_enis) = h1.await.unwrap();
    let (all_subnets, routetables) = h2.await.unwrap();
    let instances = h3.await.unwrap();

    debug!("{:?}", instances);

    let cn = ClusterNetwork::new(
        &cluster_info,
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
