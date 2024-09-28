use std::collections::HashMap;

use aws_sdk_elasticloadbalancing::Client as ELBClient;
use log::debug;

use super::shared_types::Collector;
use super::shared_types::DefaultCollector;
use super::shared_types::HypershiftCollector;
use crate::gatherer::aws::shared_types::AWSLoadBalancer;
use crate::types::MinimalClusterInfo;

pub async fn get_classic_load_balancers(
    elb_client: &ELBClient,
    cluster_info: &MinimalClusterInfo,
) -> Result<Vec<AWSLoadBalancer>, aws_sdk_elasticloadbalancing::Error> {
    let mut cluster_lbs = vec![];
    debug!("Retrieving classic LoadBalancers");
    let collector: Box<dyn Collector + Send> = match cluster_info.cluster_type {
        crate::types::ClusterType::Hypershift => {
            debug!("Using hypershift collector");
            Box::new(HypershiftCollector {})
        }
        _ => {
            debug!("Using default collector");
            Box::new(DefaultCollector {
                cluster_id: &cluster_info.cluster_id,
                cluster_infra_name: &cluster_info.cluster_infra_name,
            })
        }
    };
    let mut lb_names = HashMap::new();
    let lb_out;
    match elb_client.describe_load_balancers().send().await {
        Ok(success) => lb_out = success,
        Err(err) => return Err(aws_sdk_elasticloadbalancing::Error::from(err)),
    };
    if let Some(lbs) = lb_out.load_balancer_descriptions {
        for lb in lbs {
            let lb_name = lb.load_balancer_name.as_ref().unwrap().clone();
            lb_names.insert(lb_name, lb);
        }
    }
    for (lb_name, lb_val) in lb_names {
        debug!("Checking loadbalancer: {}", lb_name);
        let tags;
        match elb_client
            .describe_tags()
            .load_balancer_names(lb_name)
            .send()
            .await
        {
            Ok(success) => tags = success,
            Err(err) => return Err(aws_sdk_elasticloadbalancing::Error::from(err)),
        };
        if let Some(tag_descriptions) = tags.tag_descriptions {
            for td in tag_descriptions {
                if let Some(ref tag) = td.tags {
                    for t in tag {
                        debug!("Checking tag: {:?}", t);
                        if collector.match_tag(t.clone().into()) {
                            debug!("Tag matched");

                            let tags: Vec<crate::gatherer::aws::shared_types::Tag> = match td.tags {
                                None => {
                                    vec![]
                                }
                                Some(ref ts) => ts.iter().map(|t| t.clone().into()).collect(),
                            };
                            cluster_lbs
                                .push(AWSLoadBalancer::ClassicLoadBalancer((lb_val.clone(), tags)))
                        }
                    }
                }
            }
        }
    }
    return Ok(cluster_lbs);
}
