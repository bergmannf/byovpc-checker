use log::debug;
use shared_types::Tag;

pub const DEFAULT_ROUTER_TAG_HYPERSHIFT: &str = "kubernetes.io/service-name";
pub const DEFAULT_ROUTER_VALUE_HYPERSHIFT: &str = "openshift-ingress/router-default";
pub const DEFAULT_ROUTER_TAG: &str = "openshift-ingress/router-default";
pub const CLUSTER_TAG_PREFIX: &str = "kubernetes.io/cluster/";

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
