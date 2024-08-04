use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassicLoadBalancer {
    pub load_balancer_name: String,
    pub dns_name: String,
    pub vpc_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkLoadBalancer {
    pub load_balancer_arn: String,
    pub load_balancer_name: String,
    pub dns_name: String,
    pub vpc_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    /// <p>The key of the tag.</p>
    pub key: Option<String>,
    /// <p>The value of the tag.</p>
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaggedResource<T> {
    t: T,
    tags: Vec<Tag>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subnet {
    pub subnet_id: String,
    pub availability_zone: String,
    pub vpc_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IamInstanceProfile {
    pub id: String,
    pub arn: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub instance_id: String,
    pub subnet_id: String,
    pub vpc_id: String,
    pub iam_instance_profile: IamInstanceProfile,
}
