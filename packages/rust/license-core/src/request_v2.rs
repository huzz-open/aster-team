//! Explicit installation compatibility declarations. This is not authorization
//! or authenticated customer identity. Customer must implement v2 import and
//! runtime policy before switching its request generator from v1 to this format.

use serde::{Deserialize, Serialize};

use crate::{LicenseError, LicenseRequest, MachineFactor, decode_exact, validate_request};

pub const SCHEMA: &str = "aster.license-request.v2";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: String,
    pub request_id: String,
    pub product: String,
    pub product_version: String,
    pub platform: String,
    pub architecture: String,
    pub installation_id: String,
    pub machine_fingerprint_sha256: String,
    pub machine_factors: Vec<MachineFactor>,
    pub generated_at: String,
    pub license_schema: String,
    pub capability_catalog_version: u32,
    pub quota_policy_version: u32,
}

pub fn parse_request(data: &[u8]) -> Result<Request, LicenseError> {
    if data.len() > 1 << 20 {
        return Err(LicenseError::InvalidField("request_size"));
    }
    let request: Request = decode_exact(data)?;
    request.validate()?;
    Ok(request)
}

impl Request {
    pub fn validate(&self) -> Result<(), LicenseError> {
        if self.schema != SCHEMA
            || self.license_schema != crate::v2::SCHEMA
            || self.capability_catalog_version != crate::catalog::CATALOG_VERSION
            || self.quota_policy_version != crate::v2::QUOTA_POLICY_VERSION
        {
            return Err(LicenseError::InvalidField("request_compatibility"));
        }
        version(&self.product_version)?;
        let generated = crate::validate_exact_time("generated_at", &self.generated_at)?;
        let format = time::macros::format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        );
        if generated
            .format(format)
            .map_err(|_| LicenseError::InvalidField("generated_at"))?
            != self.generated_at
        {
            return Err(LicenseError::InvalidField("generated_at"));
        }
        validate_request(&LicenseRequest {
            schema: crate::LICENSE_REQUEST_SCHEMA.to_owned(),
            request_id: self.request_id.clone(),
            product: self.product.clone(),
            product_version: self.product_version.clone(),
            platform: self.platform.clone(),
            architecture: self.architecture.clone(),
            installation_id: self.installation_id.clone(),
            machine_fingerprint_sha256: self.machine_fingerprint_sha256.clone(),
            machine_factors: self.machine_factors.clone(),
            generated_at: self.generated_at.clone(),
        })
    }

    pub fn supports(&self, minimum: &str, catalog: u32, quota_policy: u32) -> bool {
        self.validate().is_ok()
            && self.capability_catalog_version == catalog
            && self.quota_policy_version == quota_policy
            && version_at_least(&self.product_version, minimum)
    }
}

fn version(value: &str) -> Result<semver::Version, LicenseError> {
    if value.len() > 64 {
        return Err(LicenseError::InvalidField("product_version"));
    }
    semver::Version::parse(value).map_err(|_| LicenseError::InvalidField("product_version"))
}

pub fn version_at_least(actual: &str, minimum: &str) -> bool {
    match (version(actual), version(minimum)) {
        (Ok(a), Ok(b)) => !a.cmp_precedence(&b).is_lt(),
        _ => false,
    }
}
