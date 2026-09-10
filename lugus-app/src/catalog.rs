use crate::{AppError, ErrorKind, FetchCommand, Operation, Result};
use lugus_financial::domain::ProviderIdentity;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, btree_map::Entry};

const SUPPORTED: [(Operation, &str, u32); 7] = [
    (Operation::Resolve, "company_resolution", 1),
    (Operation::Lookup, "company_resolution", 1),
    (Operation::Filings, "filings", 1),
    (Operation::Facts, "fundamentals", 1),
    (Operation::Document, "filings", 1),
    (Operation::Prices, "market_data", 1),
    (Operation::InstrumentLookup, "instrument_lookup", 1),
];

#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub identity: ProviderIdentity,
    pub active: bool,
    pub available: bool,
    pub capabilities: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderState {
    pub identity: ProviderIdentity,
    pub active: bool,
    pub available: bool,
    pub capabilities: BTreeMap<String, u32>,
    pub revision: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferedOperation {
    pub identity: ProviderIdentity,
    pub capability: String,
    pub version: u32,
    pub operation: Operation,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offering {
    pub revision: u64,
    operations: Vec<OfferedOperation>,
}

impl Offering {
    pub fn operations(&self) -> impl Iterator<Item = &OfferedOperation> {
        self.operations.iter()
    }

    pub fn supports(&self, instance_id: &str, operation: Operation) -> bool {
        self.find(instance_id, operation).is_some()
    }

    fn find(&self, instance_id: &str, operation: Operation) -> Option<&OfferedOperation> {
        self.operations.iter().find(|offered| {
            offered.identity.instance_id == instance_id && offered.operation == operation
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authorization {
    pub identity: ProviderIdentity,
    pub operation: Operation,
    pub capability: String,
    pub version: u32,
    pub generation: u64,
    pub catalog_revision: u64,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    revision: u64,
    providers: BTreeMap<String, ProviderState>,
}

impl Catalog {
    pub fn new(entries: Vec<ProviderEntry>) -> Result<Self> {
        let mut providers = BTreeMap::new();
        for configured in entries {
            validate_identity(&configured.identity)?;
            validate_capabilities(&configured.capabilities)?;
            let instance_id = configured.identity.instance_id.clone();
            let state = ProviderState {
                identity: configured.identity,
                active: configured.active,
                available: configured.available,
                capabilities: configured.capabilities,
                revision: 1,
                generation: 1,
            };
            match providers.entry(instance_id) {
                Entry::Vacant(slot) => {
                    slot.insert(state);
                }
                Entry::Occupied(_) => {
                    return Err(AppError::new(
                        ErrorKind::Conflict,
                        "duplicate configured provider instance ID",
                        false,
                    ));
                }
            }
        }
        Ok(Self {
            revision: 1,
            providers,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn get(&self, instance_id: &str) -> Option<&ProviderState> {
        self.providers.get(instance_id)
    }

    pub fn providers(&self) -> impl Iterator<Item = &ProviderState> {
        self.providers.values()
    }

    pub fn snapshot(&self) -> Offering {
        let operations = self
            .providers
            .values()
            .flat_map(|provider| {
                SUPPORTED
                    .into_iter()
                    .filter(move |(_, capability, version)| {
                        provider.active
                            && provider.available
                            && provider.capabilities.get(*capability) == Some(version)
                    })
                    .map(move |(operation, capability, version)| OfferedOperation {
                        identity: provider.identity.clone(),
                        capability: capability.into(),
                        version,
                        operation,
                        generation: provider.generation,
                    })
            })
            .collect();
        Offering {
            revision: self.revision,
            operations,
        }
    }

    pub fn activate(&mut self, instance_id: &str) -> Result<()> {
        self.set_active(instance_id, true)
    }

    pub fn deactivate(&mut self, instance_id: &str) -> Result<()> {
        self.set_active(instance_id, false)
    }

    pub fn set_available(&mut self, instance_id: &str, available: bool) -> Result<()> {
        if self.provider(instance_id)?.available == available {
            return Ok(());
        }
        let revision = self.next_revision()?;
        let provider = self
            .providers
            .get_mut(instance_id)
            .expect("provider was checked");
        provider.available = available;
        provider.revision = revision;
        Ok(())
    }

    pub fn restart(
        &mut self,
        identity: ProviderIdentity,
        capabilities: BTreeMap<String, u32>,
    ) -> Result<u64> {
        validate_identity(&identity)?;
        validate_capabilities(&capabilities)?;
        let instance_id = identity.instance_id.clone();
        let generation = self
            .provider(&instance_id)?
            .generation
            .checked_add(1)
            .ok_or_else(|| {
                AppError::new(
                    ErrorKind::ResourceLimit,
                    "provider generation exhausted",
                    false,
                )
            })?;
        let revision = self.next_revision()?;
        let provider = self
            .providers
            .get_mut(&instance_id)
            .expect("provider was checked");
        provider.identity = identity;
        provider.capabilities = capabilities;
        provider.available = true;
        provider.generation = generation;
        provider.revision = revision;
        Ok(generation)
    }

    pub fn authorize(&self, offering: &Offering, command: &FetchCommand) -> Result<Authorization> {
        command.validate()?;
        let instance_id = command.instance_id();
        let operation = command.operation();
        let offered = offering.find(instance_id, operation).ok_or_else(|| {
            AppError::new(
                ErrorKind::Unsupported,
                "operation was not offered for this turn",
                false,
            )
        })?;
        if (offered.capability.as_str(), offered.version) != operation.capability() {
            return Err(AppError::new(
                ErrorKind::Unsupported,
                "offering capability does not match the operation",
                false,
            ));
        }
        let current = self.provider(instance_id)?;
        if current.generation != offered.generation || current.identity != offered.identity {
            return Err(AppError::new(
                ErrorKind::StaleReference,
                "provider offering is stale",
                false,
            ));
        }
        if !current.active {
            return Err(AppError::new(
                ErrorKind::Deactivated,
                "provider is deactivated",
                false,
            ));
        }
        if !current.available {
            return Err(AppError::new(
                ErrorKind::Unavailable,
                "provider is unavailable",
                true,
            ));
        }
        if current.capabilities.get(&offered.capability) != Some(&offered.version) {
            return Err(AppError::new(
                ErrorKind::Unsupported,
                "provider capability changed",
                false,
            ));
        }
        Ok(Authorization {
            identity: current.identity.clone(),
            operation,
            capability: offered.capability.clone(),
            version: offered.version,
            generation: offered.generation,
            catalog_revision: self.revision,
        })
    }

    fn set_active(&mut self, instance_id: &str, active: bool) -> Result<()> {
        if self.provider(instance_id)?.active == active {
            return Ok(());
        }
        let revision = self.next_revision()?;
        let provider = self
            .providers
            .get_mut(instance_id)
            .expect("provider was checked");
        provider.active = active;
        provider.revision = revision;
        Ok(())
    }

    fn provider(&self, instance_id: &str) -> Result<&ProviderState> {
        self.providers.get(instance_id).ok_or_else(|| {
            AppError::new(
                ErrorKind::Unavailable,
                "provider instance is not configured",
                false,
            )
        })
    }

    fn next_revision(&mut self) -> Result<u64> {
        self.revision = self.revision.checked_add(1).ok_or_else(|| {
            AppError::new(
                ErrorKind::ResourceLimit,
                "catalog revision exhausted",
                false,
            )
        })?;
        Ok(self.revision)
    }
}

fn validate_identity(identity: &ProviderIdentity) -> Result<()> {
    let valid = [
        &identity.instance_id,
        &identity.plugin_id,
        &identity.plugin_version,
    ]
    .into_iter()
    .all(|value| {
        !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    });
    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorKind::InvalidInput,
            "invalid provider identity",
            false,
        ))
    }
}

fn validate_capabilities(capabilities: &BTreeMap<String, u32>) -> Result<()> {
    if capabilities.iter().all(|(name, version)| {
        !name.trim().is_empty()
            && name.len() <= 256
            && !name.chars().any(char::is_control)
            && *version > 0
    }) {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorKind::InvalidInput,
            "invalid provider capabilities",
            false,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn authorization_rejects_an_operation_with_mismatched_capability_metadata() {
        let identity = ProviderIdentity {
            instance_id: "provider-a".into(),
            plugin_id: "fixture".into(),
            plugin_version: "1".into(),
        };
        let catalog = Catalog::new(vec![ProviderEntry {
            identity: identity.clone(),
            active: true,
            available: true,
            capabilities: BTreeMap::from([("filings".into(), 1)]),
        }])
        .unwrap();
        let forged = Offering {
            revision: catalog.revision(),
            operations: vec![OfferedOperation {
                identity,
                capability: "filings".into(),
                version: 1,
                operation: Operation::Prices,
                generation: 1,
            }],
        };
        let command: FetchCommand = serde_json::from_value(json!({
            "operation": "prices",
            "instance_id": "provider-a",
            "query": {
                "instrument": {"namespace": "native:symbol", "value": "ACME"},
                "start": "2025-01-01", "end": "2025-01-02",
                "cursor": null, "page_size": 100
            }
        }))
        .unwrap();

        assert_eq!(
            catalog.authorize(&forged, &command).unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }
}
