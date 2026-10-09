use std::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

use indexmap::IndexMap;
use netfyr_backend::{BackendError, BackendFuture, DryRunReport, NetworkBackend};
use netfyr_reconcile::{
    ConflictIdentity, Contribution, Diagnostic, MergeOptions, ReconcileError, reconcile,
};
use netfyr_state::plan::{
    AddressObservation, DeviceObservation, InterfaceId, Ipv4Address, NamespaceId, Observation,
    OperationKind, StateDiff,
};
use netfyr_state::{Match, SchemaRegistry, Source, State, Value};

fn interface() -> InterfaceId {
    InterfaceId {
        namespace: NamespaceId {
            device: 1,
            inode: 2,
        },
        index: 7,
        generation: 3,
        name: "eth0".into(),
        device_type: "ethernet".into(),
        mac: Some("02:00:00:00:00:01".into()),
    }
}

fn observation(addresses: Vec<AddressObservation>, complete: bool) -> Observation {
    let mut state = State::new(Source::Kernel);
    state.device_type = "ethernet".into();
    state.match_spec = Match {
        name: Some("eth0".into()),
        r#type: Some("ethernet".into()),
        ..Match::default()
    };
    state.fields.insert("mtu".into(), Value::U64(1500));
    Observation {
        devices: vec![DeviceObservation {
            identity: interface(),
            state,
            addresses,
            addresses_complete: complete,
            observed_at: Instant::now(),
        }],
        diagnostics: Vec::new(),
    }
}

fn contribution(fields: IndexMap<String, Value>) -> Contribution {
    Contribution::new(
        "p",
        "p",
        Source::Static { policy: "p".into() },
        "ethernet",
        Match {
            name: Some("eth0".into()),
            ..Match::default()
        },
        fields,
        100,
    )
}

fn address(ip: Ipv4Addr, valid: Option<u64>, preferred: Option<u64>) -> Value {
    let mut map = IndexMap::new();
    map.insert("ip".into(), Value::IpNetwork((IpAddr::V4(ip), 24)));
    if let Some(valid) = valid {
        map.insert("valid_lft".into(), Value::U64(valid));
    }
    if let Some(preferred) = preferred {
        map.insert("preferred_lft".into(), Value::U64(preferred));
    }
    Value::Map(map)
}

fn address_fields(addresses: Vec<Value>) -> IndexMap<String, Value> {
    let mut ipv4 = IndexMap::new();
    ipv4.insert("addresses".into(), Value::List(addresses));
    let mut fields = IndexMap::new();
    fields.insert("ipv4".into(), Value::Map(ipv4));
    fields
}

#[test]
fn plans_exact_identity_and_backend_operations() {
    let mut fields = address_fields(vec![address(Ipv4Addr::new(192, 0, 2, 10), None, None)]);
    fields.insert("mtu".into(), Value::U64(9000));
    let plan = reconcile(
        &[contribution(fields)],
        &observation(Vec::new(), true),
        &SchemaRegistry::new(),
        &MergeOptions::default(),
    )
    .unwrap();
    assert!(
        plan.diff()
            .operations
            .iter()
            .all(|operation| operation.target == interface())
    );
    assert!(matches!(
        plan.diff().operations[0].kind,
        OperationKind::SetMtu(9000)
    ));
    assert!(matches!(
        plan.diff().operations[1].kind,
        OperationKind::AddIpv4(_)
    ));
    assert_eq!(
        plan.diff().operations[1]
            .address_group
            .as_ref()
            .unwrap()
            .addresses,
        Vec::<AddressObservation>::new()
    );
}

#[test]
fn managed_addresses_require_complete_inventory_and_valid_lifetimes() {
    let fields = address_fields(vec![address(Ipv4Addr::new(192, 0, 2, 10), None, None)]);
    assert!(matches!(
        reconcile(
            &[contribution(fields.clone())],
            &observation(Vec::new(), false),
            &SchemaRegistry::new(),
            &MergeOptions::default()
        ),
        Err(ReconcileError::IncompleteAddressInventory { .. })
    ));
    let visible = AddressObservation {
        address: Ipv4Address {
            ip: Ipv4Addr::new(192, 0, 2, 10),
            prefix_len: 24,
            valid_lft: None,
            preferred_lft: None,
        },
        flags: 0,
        scope: 0,
        peer: None,
    };
    assert!(matches!(
        reconcile(
            &[contribution(fields)],
            &observation(vec![visible], false),
            &SchemaRegistry::new(),
            &MergeOptions::default()
        ),
        Err(ReconcileError::IncompleteAddressInventory { .. })
    ));
    let invalid = address_fields(vec![address(Ipv4Addr::new(192, 0, 2, 10), Some(0), None)]);
    assert!(matches!(
        reconcile(
            &[contribution(invalid)],
            &observation(Vec::new(), true),
            &SchemaRegistry::new(),
            &MergeOptions::default()
        ),
        Err(ReconcileError::InvalidAddress { .. })
    ));
}

#[test]
fn omitted_forever_lifetimes_do_not_replace_observed_forever_address() {
    let actual = Ipv4Address {
        ip: Ipv4Addr::new(192, 0, 2, 10),
        prefix_len: 24,
        valid_lft: Some(u32::MAX),
        preferred_lft: Some(u32::MAX),
    };
    let observed = AddressObservation {
        address: actual,
        flags: 0,
        scope: 0,
        peer: None,
    };
    let plan = reconcile(
        &[contribution(address_fields(vec![address(
            Ipv4Addr::new(192, 0, 2, 10),
            None,
            None,
        )]))],
        &observation(vec![observed], true),
        &SchemaRegistry::new(),
        &MergeOptions::default(),
    )
    .unwrap();
    assert!(plan.diff().operations.is_empty());
}

#[test]
fn group_operations_have_snapshot_and_a_complete_dependency_chain() {
    let first = Ipv4Address {
        ip: Ipv4Addr::new(192, 0, 2, 10),
        prefix_len: 24,
        valid_lft: None,
        preferred_lft: None,
    };
    let second = Ipv4Address {
        ip: Ipv4Addr::new(192, 0, 2, 11),
        prefix_len: 24,
        valid_lft: None,
        preferred_lft: None,
    };
    let observed = vec![
        AddressObservation {
            address: second.clone(),
            flags: 0,
            scope: 0,
            peer: None,
        },
        AddressObservation {
            address: first.clone(),
            flags: 0,
            scope: 0,
            peer: None,
        },
    ];
    let plan = reconcile(
        &[contribution(address_fields(vec![
            address(first.ip, None, None),
            address(second.ip, None, None),
        ]))],
        &observation(observed, true),
        &SchemaRegistry::new(),
        &MergeOptions::default(),
    )
    .unwrap();
    let operations = &plan.diff().operations;
    assert_eq!(operations.len(), 4);
    assert!(operations[0].address_group.is_some());
    assert_eq!(operations[1].depends_on, vec![0]);
    assert_eq!(operations[2].depends_on, vec![1]);
    assert_eq!(operations[3].depends_on, vec![2]);
}

#[test]
fn rejects_peer_or_scope_groups_that_backend_cannot_rebuild_safely() {
    let actual = Ipv4Address {
        ip: Ipv4Addr::new(192, 0, 2, 10),
        prefix_len: 24,
        valid_lft: None,
        preferred_lft: None,
    };
    let observed = AddressObservation {
        address: actual,
        flags: 0,
        scope: 0,
        peer: Some(Ipv4Addr::new(198, 51, 100, 10)),
    };
    assert!(matches!(
        reconcile(
            &[contribution(address_fields(vec![]))],
            &observation(vec![observed], true),
            &SchemaRegistry::new(),
            &MergeOptions::default(),
        ),
        Err(ReconcileError::UnsafeAddressGroup { .. })
    ));
}

#[test]
fn keeps_structured_conflict_and_mirrors_it_to_backend_diagnostics() {
    let mut high_fields = IndexMap::new();
    high_fields.insert("mtu".into(), Value::U64(9000));
    let mut low_fields = IndexMap::new();
    low_fields.insert("mtu".into(), Value::U64(1400));
    let high = contribution(high_fields);
    let low = Contribution::new(
        "p",
        "p:provider",
        Source::Dhcp { policy: "p".into() },
        "ethernet",
        Match {
            name: Some("eth0".into()),
            ..Match::default()
        },
        low_fields,
        50,
    );
    let plan = reconcile(
        &[high, low],
        &observation(Vec::new(), true),
        &SchemaRegistry::new(),
        &MergeOptions::default(),
    )
    .unwrap();
    assert!(matches!(
        &plan.diagnostics[..],
        [Diagnostic::ValueConflict(conflict)]
            if matches!(conflict.identity, ConflictIdentity::Field { ref path } if path == "mtu")
    ));
    assert_eq!(plan.diff().diagnostics.len(), 1);
    assert_eq!(plan.provenance[0].source.as_deref(), Some("p"));
}

struct DryRunBackend;
impl NetworkBackend for DryRunBackend {
    fn namespace(&self) -> NamespaceId {
        interface().namespace
    }
    fn supported_entities(&self) -> &'static [&'static str] {
        &["ethernet"]
    }
    fn query<'a>(&'a self, _: &'a Match) -> BackendFuture<'a, Result<Observation, BackendError>> {
        Box::pin(async { Ok(observation(Vec::new(), true)) })
    }
    fn apply<'a>(&'a self, _: &'a StateDiff) -> BackendFuture<'a, netfyr_backend::ApplyReport> {
        Box::pin(async { unreachable!() })
    }
    fn dry_run<'a>(&'a self, diff: &'a StateDiff) -> BackendFuture<'a, DryRunReport> {
        Box::pin(async move {
            DryRunReport {
                changes: diff.changes.clone(),
                ..DryRunReport::default()
            }
        })
    }
}

#[tokio::test]
async fn reconciliation_output_passes_directly_to_backend_dry_run() {
    let mut fields = IndexMap::new();
    fields.insert("mtu".into(), Value::U64(9000));
    let plan = reconcile(
        &[contribution(fields)],
        &observation(Vec::new(), true),
        &SchemaRegistry::new(),
        &MergeOptions::default(),
    )
    .unwrap();
    let report = DryRunBackend.dry_run(plan.diff()).await;
    assert_eq!(report.changes, plan.diff().changes);
}
