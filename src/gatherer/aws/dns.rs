use std::error::Error;

use async_trait::async_trait;
use aws_sdk_route53::{
    types::{HostedZone, ResourceRecord},
    Client,
};
use log::{debug, error};

use crate::{
    gatherer::Gatherer,
    types::{InvariantError, MinimalClusterInfo},
};

use super::shared_types::HostedZoneWithRecords;

pub struct HostedZoneGatherer<'a> {
    pub client: &'a Client,
    pub cluster_info: &'a MinimalClusterInfo,
}

impl<'a> HostedZoneGatherer<'a> {
    async fn get_hosted_zones(&self) -> Result<Vec<HostedZone>, Box<dyn Error>> {
        let Some(ref domain) = self.cluster_info.base_domain else {
            return Err(Box::new(InvariantError {
                msg: "base_domain for cluster was empty - could not retrieve HostedZones"
                    .to_string(),
            }));
        };
        let mut cluster_zones = vec![];
        let mut paginator = self.client.list_hosted_zones().into_paginator().send();
        debug!("Fetching hosted zone for base domain: {}", domain);
        while let Some(res) = paginator.next().await {
            match res {
                Ok(zones) => {
                    for zone in zones.hosted_zones {
                        if zone.name.contains(domain) {
                            cluster_zones.push(zone)
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to fetch hosted zones: {}", e);
                    return Err(Box::new(e));
                }
            }
        }
        Ok(cluster_zones)
    }
}

#[async_trait]
impl<'a> Gatherer for HostedZoneGatherer<'a> {
    type Resource = HostedZone;

    async fn gather(&self) -> Result<Vec<Self::Resource>, Box<dyn Error>> {
        debug!("Fetching hosted zones");
        self.get_hosted_zones().await
    }
}

pub struct ResourceRecordGatherer<'a> {
    pub client: &'a Client,
    pub hosted_zones: &'a Vec<HostedZone>,
}

impl<'a> ResourceRecordGatherer<'a> {
    async fn get_resource_records(&self) -> Result<Vec<HostedZoneWithRecords>, Box<dyn Error>> {
        let mut hzrs = vec![];
        for hz in self.hosted_zones {
            debug!("Fetching resource record set for hosted zone: {}", hz.id);
            match self
                .client
                .list_resource_record_sets()
                .hosted_zone_id(&hz.id)
                .send()
                .await
            {
                Ok(r) => {
                    let hzr = HostedZoneWithRecords {
                        hosted_zone: hz.clone(),
                        resource_records: r.resource_record_sets.clone(),
                    };
                    hzrs.push(hzr);
                }
                Err(e) => {
                    error!("Failed to fetch resource records: {}", e);
                    return Err(Box::new(e));
                }
            };
        }
        Ok(hzrs)
    }
}

#[async_trait]
impl<'a> Gatherer for ResourceRecordGatherer<'a> {
    type Resource = HostedZoneWithRecords;

    async fn gather(&self) -> Result<Vec<Self::Resource>, Box<dyn Error>> {
        debug!("Fetching resource record sets");
        self.get_resource_records().await
    }
}
