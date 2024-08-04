use ::shared_types::Subnet;
use extism_pdk::*;

#[plugin_fn]
pub fn verify(Json(subnet): Json<Vec<Subnet>>) -> FnResult<String> {
    Ok(format!("Received: {:?}", subnet))
}
