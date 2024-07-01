use async_trait::async_trait;
use std::error::Error;
pub mod aws;

#[async_trait]
pub trait Gatherer {
    type Resource;
    async fn gather(&self) -> Result<Vec<Self::Resource>, Box<dyn Error>>;
}