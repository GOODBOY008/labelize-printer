//! Admission and durable restart protection during an installer handoff.
use crate::AppPaths;
use anyhow::{ensure, Result};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Admission {
    pub raw_connections: usize,
    prepared: Option<(String, Instant)>,
    committed: bool,
}
impl Admission {
    pub fn accepting(&mut self) -> bool {
        if !self.committed
            && self
                .prepared
                .as_ref()
                .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            self.prepared = None;
        }
        self.prepared.is_none()
    }
    pub fn require_accepting(&mut self) -> Result<()> {
        ensure!(
            self.accepting(),
            "An update is being prepared; try again after reopening Labelize Printer"
        );
        Ok(())
    }
    pub fn prepare(&mut self, token: String, active_jobs: bool) -> Result<()> {
        uuid::Uuid::parse_str(&token)?;
        self.require_accepting()?;
        ensure!(
            self.raw_connections == 0 && !active_jobs,
            "Wait for active print jobs and RAW connections before installing the update"
        );
        self.prepared = Some((token, Instant::now() + Duration::from_secs(120)));
        Ok(())
    }
    pub fn cancel(&mut self, token: &str) -> Result<()> {
        ensure!(!self.committed, "Update already committed");
        if let Some((owner, _)) = &self.prepared {
            ensure!(
                owner == token,
                "Update preparation belongs to another request"
            );
        }
        self.prepared = None;
        Ok(())
    }
    pub fn commit(&mut self, token: &str) -> Result<()> {
        ensure!(
            !self.accepting()
                && self
                    .prepared
                    .as_ref()
                    .is_some_and(|(owner, _)| owner == token),
            "Update preparation expired or belongs to another request"
        );
        self.committed = true;
        Ok(())
    }
}

pub fn pending(paths: &AppPaths) -> std::path::PathBuf {
    paths.root.join("update-pending.json")
}
pub fn require_ready(paths: &AppPaths) -> Result<()> {
    ensure!(!pending(paths).exists(), "An installation needs verification. Open Software update in Labelize Printer to resume printing.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_connections_are_busy_before_the_job_exists() {
        let mut gate = Admission {
            raw_connections: 1,
            ..Default::default()
        };
        assert!(gate
            .prepare(uuid::Uuid::new_v4().to_string(), false)
            .is_err());
        assert!(gate.accepting());
    }
    #[test]
    fn lease_recovery_and_ownership() {
        let mut gate = Admission::default();
        let owner = uuid::Uuid::new_v4().to_string();
        assert!(gate.prepare(owner.clone(), true).is_err());
        gate.prepare(owner.clone(), false).unwrap();
        assert!(!gate.accepting());
        assert!(gate.cancel("other").is_err());
        assert!(gate.commit("other").is_err());
        gate.prepared.as_mut().unwrap().1 = Instant::now();
        assert!(gate.accepting());
        assert!(gate.commit(&owner).is_err());
        gate.prepare(owner.clone(), false).unwrap();
        gate.cancel(&owner).unwrap();
        assert!(gate.accepting());
        gate.prepare(owner.clone(), false).unwrap();
        gate.commit(&owner).unwrap();
        gate.prepared.as_mut().unwrap().1 = Instant::now();
        assert!(!gate.accepting());
    }
}
