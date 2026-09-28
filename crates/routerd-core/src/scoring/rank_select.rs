use super::{CandidateProvider, RequestProfile, ScoredCandidate, ScoringEngine};
use crate::config::{ThresholdsConfig, TierConfig};
use crate::error::{Result, RouterError};

impl ScoringEngine {
    /// Rank candidates for a request and return sorted candidates descending by score.
    /// When the tier config names a default model and it survived scoring, it
    /// is pinned first; an ineligible default is ignored and scoring stands.
    pub fn rank_candidates(
        req: &RequestProfile,
        candidates: &[CandidateProvider],
        tier_cfg: &TierConfig,
        thresholds: &ThresholdsConfig,
    ) -> Vec<ScoredCandidate> {
        let mut scored: Vec<ScoredCandidate> = candidates
            .iter()
            .map(|c| Self::score_candidate(req, c, tier_cfg, thresholds))
            .filter(|s| !s.disqualified)
            .collect();

        scored.sort_by(|a, b| b.total_score.partial_cmp(&a.total_score).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(def) = tier_cfg.default_model.as_deref() {
            if let Some(pos) = scored.iter().position(|s| s.model_name == def) {
                let mut pinned = scored.remove(pos);
                pinned.reason = format!("{} [tier default]", pinned.reason);
                scored.insert(0, pinned);
            }
        }
        scored
    }

    /// Select the best candidate or return an error if no valid candidates exist.
    pub fn select_best(
        req: &RequestProfile,
        candidates: &[CandidateProvider],
        tier_cfg: &TierConfig,
        thresholds: &ThresholdsConfig,
    ) -> Result<ScoredCandidate> {
        let ranked = Self::rank_candidates(req, candidates, tier_cfg, thresholds);
        ranked.into_iter().next().ok_or_else(|| {
            RouterError::NoHealthyProvider(format!(
                "No candidate satisfies request (model='{}', tier='{}', tokens={})",
                req.requested_model,
                req.requested_tier,
                req.total_tokens()
            ))
        })
    }
}
