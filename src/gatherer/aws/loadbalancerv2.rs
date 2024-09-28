use std::collections::HashMap;

use async_trait::async_trait;
use aws_sdk_elasticloadbalancingv2::operation::describe_load_balancers::DescribeLoadBalancersOutput;
use aws_sdk_elasticloadbalancingv2::types::LoadBalancer;
use aws_sdk_elasticloadbalancingv2::Client as ELBv2Client;
use log::debug;
use std::error::Error;

use crate::gatherer::aws::shared_types::{Collector, DefaultCollector, HypershiftCollector};
use crate::gatherer::Gatherer;
use crate::types::MinimalClusterInfo;

use super::shared_types::AWSLoadBalancer;

pub struct LoadBalancerGatherer<'a> {
    pub client: &'a ELBv2Client,
    pub cluster_info: &'a MinimalClusterInfo,
}

#[async_trait]
impl<'a> Gatherer for LoadBalancerGatherer<'a> {
    type Resource = LoadBalancer;

    async fn gather(&self) -> Result<Vec<Self::Resource>, Box<dyn Error>> {
        debug!("Retrieving LoadBalancers");
        let mut lb_arns = HashMap::new();
        let collector: Box<dyn Collector + Send> = match self.cluster_info.cluster_type {
            crate::types::ClusterType::Hypershift => {
                debug!("Using hypershift collector");
                Box::new(HypershiftCollector {})
            }
            _ => {
                debug!("Using default collector");
                Box::new(DefaultCollector {
                    cluster_id: &self.cluster_info.cluster_id,
                    cluster_infra_name: &self.cluster_info.cluster_infra_name,
                })
            }
        };
        let mut cluster_lbs = vec![];
        let lb_out: DescribeLoadBalancersOutput;
        match self.client.describe_load_balancers().send().await {
            Ok(success) => lb_out = success,
            Err(err) => return Err(Box::new(err)),
        };
        if let Some(lbs) = lb_out.load_balancers {
            for lb in lbs {
                let arn = lb.load_balancer_arn.as_ref().unwrap().clone();
                lb_arns.insert(arn, lb);
            }
        }
        for (lb_key, lb_val) in lb_arns {
            debug!("Checking loadbalancer: {}", lb_key);
            let tags;
            match self
                .client
                .describe_tags()
                .resource_arns(lb_key)
                .send()
                .await
            {
                Ok(success) => tags = success,
                Err(err) => return Err(Box::new(err)),
            };
            if let Some(tag_descriptions) = tags.tag_descriptions {
                for td in tag_descriptions {
                    if let Some(tag) = td.tags {
                        for t in tag {
                            debug!("Checking tag: {:?}", t);
                            if collector.match_tag(t.into()) {
                                debug!("Tag matched");
                                cluster_lbs.push(lb_val.clone())
                            }
                        }
                    }
                }
            }
        }
        Ok(cluster_lbs)
    }
}
