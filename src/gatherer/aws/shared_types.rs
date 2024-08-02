use aws_sdk_ec2::types::Instance;
use aws_sdk_ec2::types::SecurityGroup;
use aws_sdk_elasticloadbalancing::types::LoadBalancerDescription;
use aws_sdk_elasticloadbalancing::types::Tag as TagV1;
use aws_sdk_elasticloadbalancingv2::types::LoadBalancer;
use aws_sdk_elasticloadbalancingv2::types::Tag as TagV2;
use log::debug;
use serde::Deserialize;
use serde::Serialize;

pub const DEFAULT_ROUTER_TAG_HYPERSHIFT: &str = "kubernetes.io/service-name";
pub const DEFAULT_ROUTER_VALUE_HYPERSHIFT: &str = "openshift-ingress/router-default";
pub const DEFAULT_ROUTER_TAG: &str = "openshift-ingress/router-default";
pub const CLUSTER_TAG_PREFIX: &str = "kubernetes.io/cluster/";

// Abstracts over classic and modern loadbalancers where needed.
// Allows the method to dispatch using match where needed.
#[derive(Debug)]
pub enum AWSLoadBalancer {
    ClassicLoadBalancer(LoadBalancerDescription),
    ModernLoadBalancer(LoadBalancer),
}

#[derive(Debug)]
pub struct Tag {
    /// <p>The key of the tag.</p>
    pub key: Option<String>,
    /// <p>The value of the tag.</p>
    pub value: Option<String>,
}

impl From<TagV1> for Tag {
    fn from(value: TagV1) -> Self {
        Tag {
            key: Some(value.key),
            value: value.value,
        }
    }
}

impl From<TagV2> for Tag {
    fn from(value: TagV2) -> Self {
        Tag {
            key: value.key,
            value: value.value,
        }
    }
}

pub trait Collector {
    fn match_tag(&self, t: Tag) -> bool;
}

pub struct HypershiftCollector;

pub struct DefaultCollector<'a> {
    pub cluster_id: &'a String,
    pub cluster_infra_name: &'a String,
}

impl Collector for HypershiftCollector {
    fn match_tag(&self, t: Tag) -> bool {
        debug!(
            "Checking if {:?} matches {} with value {}",
            t, DEFAULT_ROUTER_TAG_HYPERSHIFT, DEFAULT_ROUTER_VALUE_HYPERSHIFT
        );
        t.key.is_some_and(|t| t == DEFAULT_ROUTER_TAG_HYPERSHIFT)
            && t.value
                .is_some_and(|t| t == DEFAULT_ROUTER_VALUE_HYPERSHIFT)
    }
}

impl<'a> Collector for DefaultCollector<'a> {
    fn match_tag(&self, t: Tag) -> bool {
        let cluster_id_tag = format!("{}{}", CLUSTER_TAG_PREFIX, self.cluster_id);
        let cluster_name_tag = format!("{}{}", CLUSTER_TAG_PREFIX, self.cluster_infra_name);
        debug!(
            "Checking if {:?} matches {} or {}",
            t, cluster_id_tag, cluster_name_tag
        );
        t.key
            .is_some_and(|t| t.contains(&cluster_id_tag) || t.contains(&cluster_name_tag))
            && t.value.is_some_and(|t| t == "owned" || t == "shared")
    }
}

pub struct AWSInstance {
    pub instance: Instance,
    pub security_groups: Vec<SecurityGroup>,
}

pub struct TaggedResource<T> {
    t: T,
    tags: Vec<Tag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subnet {
    pub subnet_id: String,
    pub availibility_zone: String,
}

impl From<aws_sdk_ec2::types::Subnet> for Subnet {
    fn from(value: aws_sdk_ec2::types::Subnet) -> Self {
        Self {
            subnet_id: value.subnet_id().unwrap().to_string(),
            availibility_zone: value.availability_zone.unwrap().to_string(),
        }
    }
}
