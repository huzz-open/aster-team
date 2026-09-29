use super::*;

/// Persisted before dispatching either candidate service start. The digest binds
/// the private slot material without placing credentials in this journal.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateStartup {
    pub material_sha256: String,
    pub clock: UpgradeClock,
}

impl CandidateStartup {
    pub const BUDGET_MS: u64 = 90_000;

    pub(super) fn valid_for(&self, intent: &PreparationIntent) -> bool {
        crate::valid_checksum(&self.material_sha256)
            && self.clock.boot_id == intent.clock.boot_id
            && self.clock.uptime_ms >= intent.clock.uptime_ms
            && self.clock.uptime_ms.checked_add(Self::BUDGET_MS).is_some()
    }

    #[must_use]
    pub fn accepts_observation(&self, material_sha256: &str, clock: &UpgradeClock) -> bool {
        crate::valid_checksum(&self.material_sha256)
            && self.material_sha256 == material_sha256
            && clock.boot_id == self.clock.boot_id
            && clock.uptime_ms >= self.clock.uptime_ms
            && self
                .clock
                .uptime_ms
                .checked_add(Self::BUDGET_MS)
                .is_some_and(|end| clock.uptime_ms < end)
    }
}

/// Captured after both services start, while candidate admission is still closed.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateActivation {
    pub control: RuntimeSnapshot,
    pub runner: ServiceInvocation,
}

impl CandidateActivation {
    pub(super) fn valid_for(
        &self,
        intent: &PreparationIntent,
        candidate: &ActiveReleaseSlot,
    ) -> bool {
        let control = &self.control;
        let Some(service) = &control.service else {
            return false;
        };
        let Some(previous) = &intent.previous_process.service else {
            return false;
        };
        let services = [
            previous,
            &intent.previous_runner_process,
            service,
            &self.runner,
        ];
        valid_control(control, intent, candidate) && distinct_services(&services)
    }
}

pub(super) fn valid_control(
    control: &RuntimeSnapshot,
    intent: &PreparationIntent,
    candidate: &ActiveReleaseSlot,
) -> bool {
    control.same_process(control)
        && crate::valid_identifier(&control.instance_id)
        && control.installation_id == intent.previous_process.installation_id
        && control.instance_id != intent.previous_process.instance_id
        && control.slot == candidate.slot
        && control.product_version == candidate.version
        && !control.lifecycle.accepting
        && !control.lifecycle.stopping
        && control.lifecycle.in_flight == 0
        && control.lifecycle.revision < u64::MAX
}

pub(super) fn distinct_services(services: &[&ServiceInvocation]) -> bool {
    services.iter().enumerate().all(|(index, item)| {
        item.valid()
            && services[..index].iter().all(|other| {
                item.process_id != other.process_id && item.invocation_id != other.invocation_id
            })
    })
}
