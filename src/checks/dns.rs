use aws_sdk_route53::types::ResourceRecordSet;
use derive_builder::Builder;

use crate::{
    gatherer::aws::shared_types::{AWSLoadBalancer, HostedZoneWithRecords},
    types::{VerificationResult, Verifier},
};

#[derive(Builder)]
pub struct HostedZoneChecks {
    pub hosted_zones: Vec<HostedZoneWithRecords>,
    pub load_balancers: Vec<AWSLoadBalancer>,
}

impl HostedZoneChecks {
    fn get_resource_record_targets(&self) -> Vec<(String, String)> {
        let resource_record_sets: Vec<ResourceRecordSet> = self
            .hosted_zones
            .iter()
            .map(|h| h.resource_records.clone())
            .flatten()
            .collect();
        let resource_values: Vec<(String, String)> = resource_record_sets
            .iter()
            .map(|r| {
                r.alias_target
                    .clone()
                    .map(|at| (r.name.clone(), at.dns_name.clone()))
            })
            .flatten()
            .collect();
        resource_values
    }

    fn get_load_balancer_names(&self) -> Vec<String> {
        self.load_balancers
            .iter()
            .map(|l| match l {
                AWSLoadBalancer::ClassicLoadBalancer(c) => {
                    c.dns_name.clone().unwrap_or("".to_string())
                }
                AWSLoadBalancer::ModernLoadBalancer(m) => {
                    m.dns_name.clone().unwrap_or("".to_string())
                }
            })
            .collect()
    }

    pub fn verify_number_of_hosted_zones(&self) -> VerificationResult {
        match self.hosted_zones.len() {
            0 | 1 => VerificationResult {
                message: format!("Too few hosted zones found: {}", self.hosted_zones.len()),
                severity: crate::types::Severity::Critical,
            },
            2 => VerificationResult {
                message: "Expected number of hosted zones found: 2".to_string(),
                severity: crate::types::Severity::Ok,
            },
            _ => VerificationResult {
                message: format!("Too many hosted zones found: {}", self.hosted_zones.len()),
                severity: crate::types::Severity::Critical,
            },
        }
    }

    pub fn verify_load_balancers_are_used(&self) -> Vec<VerificationResult> {
        let mut results = vec![];
        let resource_targets = self.get_resource_record_targets();
        let load_balancer_names: Vec<String> = self.get_load_balancer_names();
        for lb in load_balancer_names {
            if !resource_targets
                .iter()
                .any(|(_, target)| target.contains(&lb))
            {
                results.push(VerificationResult {
                    message: format!("LoadBalancer '{}' is not being used in any hosted zone", lb),
                    severity: crate::types::Severity::Warning,
                })
            } else {
                if let Some((name, _)) = resource_targets
                    .iter()
                    .find(|(_, target)| target.contains(&lb))
                {
                    results.push(VerificationResult {
                        message: format!("LoadBalancer {} is used in record {}", lb, name),
                        severity: crate::types::Severity::Ok,
                    })
                }
            }
        }
        results
    }

    pub fn verify_only_known_load_balancers_are_used(&self) -> Vec<VerificationResult> {
        let mut results = vec![];
        let resource_targets = self.get_resource_record_targets();
        let load_balancer_names: Vec<String> = self.get_load_balancer_names();
        for (name, target) in resource_targets {
            if !load_balancer_names.iter().any(|lb| target.contains(lb)) {
                results.push(VerificationResult {
                    message: format!("ResourceRecord '{}' is using a LoadBalancer not associated with the cluster: {}", name, target),
                    severity: crate::types::Severity::Warning,
                })
            }
        }
        results
    }
}

impl Verifier for HostedZoneChecks {
    fn verify(&self) -> Vec<crate::types::VerificationResult> {
        let mut result = self.verify_load_balancers_are_used();
        result.push(self.verify_number_of_hosted_zones());
        let mut r2 = self.verify_only_known_load_balancers_are_used();
        result.append(&mut r2);
        result
    }
}
