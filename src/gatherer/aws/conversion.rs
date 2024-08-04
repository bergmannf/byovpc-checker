use std::ops::Deref;

use aws_sdk_ec2::types::Instance as AInstance;
use aws_sdk_ec2::types::SecurityGroup;
use aws_sdk_elasticloadbalancing::types::LoadBalancerDescription;
use aws_sdk_elasticloadbalancing::types::Tag as TagV1;
use aws_sdk_elasticloadbalancingv2::types::Tag as TagV2;

use ::shared_types::*;

// Abstracts over classic and modern loadbalancers where needed.
// Allows the method to dispatch using match where needed.
#[derive(Debug)]
pub enum AWSLoadBalancer {
    ClassicLoadBalancer(LoadBalancerDescription),
    ModernLoadBalancer(aws_sdk_elasticloadbalancingv2::types::LoadBalancer),
}

pub struct AWSInstance {
    pub instance: AInstance,
    pub security_groups: Vec<SecurityGroup>,
}

pub struct ClassicLoadBalancerProxy(ClassicLoadBalancer);

impl Deref for ClassicLoadBalancerProxy {
    type Target = ClassicLoadBalancer;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<aws_sdk_elasticloadbalancing::types::LoadBalancerDescription>
    for ClassicLoadBalancerProxy
{
    fn from(value: aws_sdk_elasticloadbalancing::types::LoadBalancerDescription) -> Self {
        Self(ClassicLoadBalancer {
            load_balancer_name: value.load_balancer_name().unwrap().to_string(),
            dns_name: value.dns_name().unwrap().to_string(),
            vpc_id: value.vpc_id().unwrap().to_string(),
        })
    }
}

pub struct NetworkLoadBalancerProxy(NetworkLoadBalancer);

impl From<aws_sdk_elasticloadbalancingv2::types::LoadBalancer> for NetworkLoadBalancerProxy {
    fn from(value: aws_sdk_elasticloadbalancingv2::types::LoadBalancer) -> Self {
        Self(NetworkLoadBalancer {
            load_balancer_arn: value.load_balancer_arn().unwrap().to_string(),
            load_balancer_name: value.load_balancer_name().unwrap().to_string(),
            dns_name: value.dns_name().unwrap().to_string(),
            vpc_id: value.vpc_id().unwrap().to_string(),
        })
    }
}

pub struct TagProxy(Tag);

impl Deref for TagProxy {
    type Target = Tag;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<TagV1> for TagProxy {
    fn from(value: TagV1) -> Self {
        TagProxy(Tag {
            key: Some(value.key),
            value: value.value,
        })
    }
}

impl From<TagV2> for TagProxy {
    fn from(value: TagV2) -> Self {
        Self(Tag {
            key: value.key,
            value: value.value,
        })
    }
}

pub struct SubnetProxy(Subnet);

impl Deref for SubnetProxy {
    type Target = Subnet;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<aws_sdk_ec2::types::Subnet> for SubnetProxy {
    fn from(value: aws_sdk_ec2::types::Subnet) -> Self {
        Self(Subnet {
            subnet_id: value.subnet_id().unwrap().to_string(),
            availability_zone: value.availability_zone().unwrap().to_string(),
            vpc_id: value.vpc_id().unwrap().to_string(),
        })
    }
}

pub struct IamInstanceProfileProxy(IamInstanceProfile);

impl Deref for IamInstanceProfileProxy {
    type Target = IamInstanceProfile;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<aws_sdk_ec2::types::IamInstanceProfile> for IamInstanceProfileProxy {
    fn from(value: aws_sdk_ec2::types::IamInstanceProfile) -> Self {
        Self(IamInstanceProfile {
            id: value.id().unwrap().to_string(),
            arn: value.arn().unwrap().to_string(),
        })
    }
}

pub struct InstanceProxy(Instance);

impl From<aws_sdk_ec2::types::Instance> for InstanceProxy {
    fn from(value: aws_sdk_ec2::types::Instance) -> Self {
        Self(Instance {
            instance_id: value.instance_id().unwrap().to_string(),
            subnet_id: value.subnet_id().unwrap().to_string(),
            vpc_id: value.vpc_id().unwrap().to_string(),
            iam_instance_profile: (*IamInstanceProfileProxy::from(
                value.iam_instance_profile().unwrap().clone(),
            ))
            .clone(),
        })
    }
}
