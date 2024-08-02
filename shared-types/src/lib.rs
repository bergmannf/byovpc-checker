use aws_sdk_ec2::types::Instance;
use aws_sdk_ec2::types::SecurityGroup;
use aws_sdk_elasticloadbalancing::types::LoadBalancerDescription;
use aws_sdk_elasticloadbalancing::types::Tag as TagV1;
use aws_sdk_elasticloadbalancingv2::types::LoadBalancer;
use aws_sdk_elasticloadbalancingv2::types::Tag as TagV2;

use serde::Deserialize;
use serde::Serialize;

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
