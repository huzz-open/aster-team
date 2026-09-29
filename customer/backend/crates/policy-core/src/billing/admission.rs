//! Per-member in-flight accounting. The backend is replaceable so a future
//! multi-instance deployment can provide shared coordination without changing
//! the admission contract.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use super::{AdmissionDecision, AdmissionSnapshot, BillingError, Money};

pub trait InFlightTracker: Send + Sync {
    /// The balance is read by the caller immediately before each attempt. A
    /// queued caller must read it again, along with the current model price.
    fn try_acquire(
        &self,
        member_id: &str,
        balance: Money,
        request_reference: Money,
        max_parallel: Option<u32>,
    ) -> Result<TrackerDecision, BillingError>;

    fn release(&self, permit_id: u64);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackerDecision {
    Acquired(u64),
    Queue,
    RejectInsufficientBalance,
}

pub enum AdmissionOutcome {
    Acquired(InFlightPermit),
    Queue,
    RejectInsufficientBalance,
}

pub struct AdmissionCoordinator {
    tracker: Arc<dyn InFlightTracker>,
}

impl AdmissionCoordinator {
    pub fn new(tracker: Arc<dyn InFlightTracker>) -> Self {
        Self { tracker }
    }

    pub fn try_acquire(
        &self,
        member_id: &str,
        balance: Money,
        request_reference: Money,
        max_parallel: Option<u32>,
    ) -> Result<AdmissionOutcome, BillingError> {
        match self
            .tracker
            .try_acquire(member_id, balance, request_reference, max_parallel)?
        {
            TrackerDecision::Acquired(id) => Ok(AdmissionOutcome::Acquired(InFlightPermit {
                tracker: Arc::clone(&self.tracker),
                id,
            })),
            TrackerDecision::Queue => Ok(AdmissionOutcome::Queue),
            TrackerDecision::RejectInsufficientBalance => {
                Ok(AdmissionOutcome::RejectInsufficientBalance)
            }
        }
    }
}

/// Dropping a permit releases its reference amount even on cancellation.
pub struct InFlightPermit {
    tracker: Arc<dyn InFlightTracker>,
    id: u64,
}

impl std::fmt::Debug for InFlightPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InFlightPermit")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for InFlightPermit {
    fn drop(&mut self) {
        self.tracker.release(self.id);
    }
}

#[derive(Default)]
pub struct MemoryInFlightTracker {
    state: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    next_id: u64,
    members: HashMap<String, HashMap<u64, Money>>,
    permit_members: HashMap<u64, String>,
}

impl InFlightTracker for MemoryInFlightTracker {
    fn try_acquire(
        &self,
        member_id: &str,
        balance: Money,
        request_reference: Money,
        max_parallel: Option<u32>,
    ) -> Result<TrackerDecision, BillingError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| BillingError::TrackerUnavailable)?;
        let active = state.members.get(member_id);
        let in_flight_count = active.map_or(0, |entries| entries.len());
        let count = u32::try_from(in_flight_count).map_err(|_| BillingError::Overflow)?;
        let mut reference = Money::zero(balance.currency);
        if let Some(entries) = active {
            for amount in entries.values() {
                reference = reference.checked_add(*amount)?;
            }
        }
        let decision = AdmissionSnapshot {
            balance,
            in_flight_reference: reference,
            in_flight_count: count,
            max_parallel,
        }
        .decide(request_reference)?;
        match decision {
            AdmissionDecision::Admit => {
                let id = state.next_id.checked_add(1).ok_or(BillingError::Overflow)?;
                state.next_id = id;
                state.permit_members.insert(id, member_id.to_owned());
                state
                    .members
                    .entry(member_id.to_owned())
                    .or_default()
                    .insert(id, request_reference);
                Ok(TrackerDecision::Acquired(id))
            }
            AdmissionDecision::Queue => Ok(TrackerDecision::Queue),
            AdmissionDecision::RejectInsufficientBalance => {
                Ok(TrackerDecision::RejectInsufficientBalance)
            }
        }
    }

    fn release(&self, permit_id: u64) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let Some(member_id) = state.permit_members.remove(&permit_id) else {
            return;
        };
        let Some(entries) = state.members.get_mut(&member_id) else {
            return;
        };
        entries.remove(&permit_id);
        if entries.is_empty() {
            state.members.remove(&member_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::Currency;
    use std::sync::Barrier;

    #[test]
    fn low_balance_serializes_and_release_rechecks_the_new_balance() {
        let coordinator = AdmissionCoordinator::new(Arc::new(MemoryInFlightTracker::default()));
        let currency = Currency::Usd;
        let reference = Money {
            currency,
            nanos: 1_000,
        };
        let first = coordinator
            .try_acquire(
                "member-a",
                Money {
                    currency,
                    nanos: 500,
                },
                reference,
                None,
            )
            .unwrap();
        let AdmissionOutcome::Acquired(first) = first else {
            panic!("first request must start")
        };
        assert!(matches!(
            coordinator
                .try_acquire(
                    "member-a",
                    Money {
                        currency,
                        nanos: 500
                    },
                    reference,
                    None
                )
                .unwrap(),
            AdmissionOutcome::Queue
        ));
        assert!(matches!(
            coordinator
                .try_acquire(
                    "member-b",
                    Money {
                        currency,
                        nanos: 500
                    },
                    reference,
                    None
                )
                .unwrap(),
            AdmissionOutcome::Acquired(_)
        ));
        drop(first);
        assert!(matches!(
            coordinator
                .try_acquire(
                    "member-a",
                    Money {
                        currency,
                        nanos: -1
                    },
                    reference,
                    None
                )
                .unwrap(),
            AdmissionOutcome::RejectInsufficientBalance
        ));
    }

    #[test]
    fn parallel_limit_and_exposure_are_atomic_per_member() {
        let coordinator = AdmissionCoordinator::new(Arc::new(MemoryInFlightTracker::default()));
        let currency = Currency::Cny;
        let reference = Money {
            currency,
            nanos: 100,
        };
        let AdmissionOutcome::Acquired(first) = coordinator
            .try_acquire(
                "member",
                Money {
                    currency,
                    nanos: 300,
                },
                reference,
                Some(2),
            )
            .unwrap()
        else {
            panic!("first")
        };
        let AdmissionOutcome::Acquired(second) = coordinator
            .try_acquire(
                "member",
                Money {
                    currency,
                    nanos: 300,
                },
                reference,
                Some(2),
            )
            .unwrap()
        else {
            panic!("second")
        };
        assert!(matches!(
            coordinator
                .try_acquire(
                    "member",
                    Money {
                        currency,
                        nanos: 300
                    },
                    reference,
                    Some(2)
                )
                .unwrap(),
            AdmissionOutcome::Queue
        ));
        drop(second);
        assert!(matches!(
            coordinator
                .try_acquire(
                    "member",
                    Money {
                        currency,
                        nanos: 150
                    },
                    reference,
                    Some(2)
                )
                .unwrap(),
            AdmissionOutcome::Queue
        ));
        drop(first);
        assert!(matches!(
            coordinator
                .try_acquire(
                    "member",
                    Money {
                        currency,
                        nanos: 150
                    },
                    reference,
                    Some(2)
                )
                .unwrap(),
            AdmissionOutcome::Acquired(_)
        ));
    }

    #[test]
    fn simultaneous_attempts_cannot_both_claim_one_members_exposure() {
        let coordinator = Arc::new(AdmissionCoordinator::new(Arc::new(
            MemoryInFlightTracker::default(),
        )));
        let start = Arc::new(Barrier::new(3));
        let finish = Arc::new(Barrier::new(3));
        let threads = (0..2)
            .map(|_| {
                let coordinator = Arc::clone(&coordinator);
                let start = Arc::clone(&start);
                let finish = Arc::clone(&finish);
                std::thread::spawn(move || {
                    start.wait();
                    let outcome = coordinator
                        .try_acquire(
                            "member",
                            Money {
                                currency: Currency::Usd,
                                nanos: 150,
                            },
                            Money {
                                currency: Currency::Usd,
                                nanos: 100,
                            },
                            None,
                        )
                        .unwrap();
                    let acquired = matches!(outcome, AdmissionOutcome::Acquired(_));
                    finish.wait();
                    acquired
                })
            })
            .collect::<Vec<_>>();
        start.wait();
        finish.wait();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .filter(|acquired| *acquired)
                .count(),
            1
        );
    }
}
