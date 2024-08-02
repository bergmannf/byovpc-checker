use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subnet {
    pub subnet_id: String,
    pub availibility_zone: String,
}

#[plugin_fn]
pub fn verify(Json(subnet): Json<Vec<Subnet>>) -> FnResult<String> {
    Ok(format!("Received: {:?}", subnet))
}
