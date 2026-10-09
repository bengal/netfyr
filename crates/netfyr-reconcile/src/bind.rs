use std::collections::{BTreeMap, BTreeSet};

use netfyr_state::plan::{DeviceObservation, Observation};

use crate::{BindingConflict, ConflictingPolicy, Contribution, UnmatchedPolicy};

pub(super) struct BoundDevice<'a> {
    pub observation: &'a DeviceObservation,
    pub policy: String,
}

pub(super) struct Binding<'a> {
    pub devices: Vec<BoundDevice<'a>>,
    pub conflicts: Vec<BindingConflict>,
    pub unmatched: Vec<UnmatchedPolicy>,
}

pub(super) fn policy_contributions<'a>(
    contributions: &'a [Contribution],
    policy: &str,
) -> Vec<&'a Contribution> {
    let mut selected: Vec<_> = contributions
        .iter()
        .filter(|item| item.policy == policy)
        .collect();
    selected.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.key.cmp(&right.key))
    });
    selected
}

pub(super) fn bind<'a>(
    contributions: &[Contribution],
    observation: &'a Observation,
) -> Binding<'a> {
    let mut selectors = BTreeMap::new();
    let mut ordered: Vec<_> = contributions.iter().collect();
    ordered.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.key.cmp(&right.key))
    });
    for contribution in ordered {
        selectors
            .entry(contribution.policy.clone())
            .or_insert_with(|| contribution.match_spec.clone());
    }
    let mut devices = Vec::new();
    let mut conflicts = Vec::new();
    let mut matched = BTreeSet::new();
    for device in &observation.devices {
        let owners: Vec<_> = selectors
            .iter()
            .filter(|(_, selector)| selector.matches(&device.state.match_spec))
            .map(|(policy, _)| policy.clone())
            .collect();
        match owners.as_slice() {
            [] => {}
            [policy] => {
                matched.insert(policy.clone());
                devices.push(BoundDevice {
                    observation: device,
                    policy: policy.clone(),
                });
            }
            _ => {
                matched.extend(owners.iter().cloned());
                conflicts.push(BindingConflict {
                    target: device.identity.clone(),
                    policies: owners
                        .into_iter()
                        .map(|name| ConflictingPolicy {
                            selector: selectors[&name].clone(),
                            name,
                        })
                        .collect(),
                });
            }
        }
    }
    devices.sort_by_key(|device| {
        (
            device.observation.identity.namespace.device,
            device.observation.identity.namespace.inode,
            device.observation.identity.index,
        )
    });
    conflicts.sort_by_key(|conflict| {
        (
            conflict.target.namespace.device,
            conflict.target.namespace.inode,
            conflict.target.index,
        )
    });
    Binding {
        devices,
        conflicts,
        unmatched: selectors
            .into_iter()
            .filter(|(policy, _)| !matched.contains(policy))
            .map(|(policy, selector)| UnmatchedPolicy { policy, selector })
            .collect(),
    }
}
