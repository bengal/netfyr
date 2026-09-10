//! The `Source` enum: what produced a `State`.

/// What produced a [`crate::State`].
///
/// Each `State` is exactly one source's contribution to one device, so the
/// source lives on the state. The default precedence
/// `Static > Vpn > Dhcp > Ra > Kernel` (see [`Source::default_priority`])
/// must hold; the exact numbers are a recommendation and may be overridden
/// per state by the policy or provider that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The static fields of a user policy. Default priority 100.
    Static {
        /// Identifies which policy produced the contribution.
        policy: String,
    },
    /// A VPN provider declared by a policy. Default priority 75.
    Vpn {
        /// Identifies which policy declared the provider.
        policy: String,
    },
    /// A DHCP provider declared by a policy. Default priority 50.
    Dhcp {
        /// Identifies which policy declared the provider.
        policy: String,
    },
    /// An IPv6 router-advertisement (SLAAC) provider declared by a policy.
    /// Default priority 25.
    Ra {
        /// Identifies which policy declared the provider.
        policy: String,
    },
    /// Read from the running kernel. Default priority 0: the baseline
    /// every provider establishes its changes on top of.
    Kernel,
}

impl Source {
    /// The default conflict-resolution priority for this source kind
    /// (higher wins). The producing policy or provider may override it on a
    /// [`crate::State`].
    pub fn default_priority(&self) -> i32 {
        match self {
            Source::Static { .. } => 100,
            Source::Vpn { .. } => 75,
            Source::Dhcp { .. } => 50,
            Source::Ra { .. } => 25,
            Source::Kernel => 0,
        }
    }

    /// The originating policy identifier, if this source is policy- or
    /// provider-backed (`None` for [`Source::Kernel`]).
    pub fn policy(&self) -> Option<&str> {
        match self {
            Source::Static { policy }
            | Source::Vpn { policy }
            | Source::Dhcp { policy }
            | Source::Ra { policy } => Some(policy),
            Source::Kernel => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Source;

    #[test]
    fn default_priority_exact_values() {
        assert_eq!(
            Source::Static { policy: "p".into() }.default_priority(),
            100
        );
        assert_eq!(Source::Vpn { policy: "p".into() }.default_priority(), 75);
        assert_eq!(Source::Dhcp { policy: "p".into() }.default_priority(), 50);
        assert_eq!(Source::Ra { policy: "p".into() }.default_priority(), 25);
        assert_eq!(Source::Kernel.default_priority(), 0);
    }

    #[test]
    fn precedence_order_static_vpn_dhcp_ra_kernel() {
        // The spec's invariant: the ordering is the requirement (the numbers a
        // recommendation); both are pinned here.
        let static_ = Source::Static {
            policy: String::new(),
        };
        let vpn = Source::Vpn {
            policy: String::new(),
        };
        let dhcp = Source::Dhcp {
            policy: String::new(),
        };
        let ra = Source::Ra {
            policy: String::new(),
        };
        let kernel = Source::Kernel;
        assert!(static_.default_priority() > vpn.default_priority());
        assert!(vpn.default_priority() > dhcp.default_priority());
        assert!(dhcp.default_priority() > ra.default_priority());
        assert!(ra.default_priority() > kernel.default_priority());
    }

    #[test]
    fn policy_accessor() {
        assert_eq!(Source::Static { policy: "a".into() }.policy(), Some("a"));
        assert_eq!(Source::Vpn { policy: "b".into() }.policy(), Some("b"));
        assert_eq!(Source::Dhcp { policy: "c".into() }.policy(), Some("c"));
        assert_eq!(Source::Ra { policy: "d".into() }.policy(), Some("d"));
        assert_eq!(Source::Kernel.policy(), None);
        // Distinct policies are distinct sources.
        assert_ne!(
            Source::Static { policy: "a".into() },
            Source::Static { policy: "b".into() }
        );
    }
}
