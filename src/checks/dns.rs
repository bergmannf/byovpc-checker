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
    pub fn verify_number_of_hosted_zones(&self) -> VerificationResult {
        match self.hosted_zones.len() {
            0 | 1 => VerificationResult::HostedZoneTooFew(self.hosted_zones.len().to_string()),
            2 => {
                VerificationResult::Success("Expected number of hosted zones found: 2".to_string())
            }
            _ => VerificationResult::HostedZoneTooMany(self.hosted_zones.len().to_string()),
        }
    }
}

impl Verifier for HostedZoneChecks {
    fn verify(&self) -> Vec<crate::types::VerificationResult> {
        return vec![self.verify_number_of_hosted_zones()];
    }
}
