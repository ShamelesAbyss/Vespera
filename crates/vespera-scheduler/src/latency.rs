use std::collections::VecDeque;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use vespera_core::{ModelId, ResourceRequest, ResourceUsage};
use vespera_model_registry::ModelRegistration;
use vespera_providers::InferenceProvider;

use crate::{InferenceMemory, ResourceScheduler, ScheduledInference, lock};

const MAX_OBSERVATIONS: usize = 128;
const RECENT_SAMPLES: usize = 3;
const MAX_SAMPLE_AGE: Duration = Duration::from_secs(15 * 60);

struct Observation {
    model_id: ModelId,
    name: String,
    provider: Weak<dyn InferenceProvider>,
    context: u32,
    output_limit: u32,
    elapsed: Duration,
    observed_at: Instant,
}

#[derive(Default)]
pub(super) struct LatencyHistory(VecDeque<Observation>);

impl LatencyHistory {
    fn estimate(
        &self,
        model: &ModelRegistration,
        provider: &Arc<dyn InferenceProvider>,
        context: u32,
        output_limit: u32,
        now: Instant,
    ) -> Option<Duration> {
        let provider = Arc::downgrade(provider);
        let mut samples: Vec<_> = self
            .0
            .iter()
            .rev()
            .filter(|sample| {
                sample.model_id == model.model_id
                    && sample.name == model.provider_model_identifier
                    && Weak::ptr_eq(&sample.provider, &provider)
                    && sample.context == context
                    && sample.output_limit == output_limit
                    && now.duration_since(sample.observed_at) <= MAX_SAMPLE_AGE
            })
            .take(RECENT_SAMPLES)
            .map(|sample| sample.elapsed)
            .collect();
        samples.sort_unstable();
        samples.get(samples.len() / 2).copied()
    }
}

impl ResourceScheduler {
    /// Record locally timed provider work after the managed task accepts its result.
    pub fn record_inference_latency(
        &self,
        inference: &ScheduledInference,
        output_limit: u32,
        elapsed: Duration,
    ) {
        if !self.preferences.prefer_observed_latency {
            return;
        }
        let mut history = lock(&self.latency);
        if history.0.len() == MAX_OBSERVATIONS {
            history.0.pop_front();
        }
        history.0.push_back(Observation {
            model_id: inference.model_id,
            name: inference.model_name().to_owned(),
            provider: Arc::downgrade(&inference.provider),
            context: inference.context_tokens,
            output_limit,
            elapsed,
            observed_at: Instant::now(),
        });
    }

    /// Compare recent observed provider call times for already eligible models.
    /// Return no preference when evidence is incomplete or request shapes differ.
    pub async fn prefer_lower_latency(
        &self,
        candidates: &[ModelRegistration],
        requested_context: Option<u32>,
        output_limit: u32,
    ) -> Option<ModelId> {
        if !self.preferences.prefer_observed_latency
            || !(2..=MAX_OBSERVATIONS).contains(&candidates.len())
        {
            return None;
        }
        let default_context = |model: &ModelRegistration| {
            model
                .context_window
                .unwrap_or(self.max_context_tokens())
                .min(self.max_context_tokens())
        };
        let context = requested_context.unwrap_or_else(|| default_context(&candidates[0]));
        let overhead = InferenceMemory::for_context(self.overhead, context).ok()?;
        let mut providers = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if (requested_context.is_none() && default_context(candidate) != context)
                || candidate.context_window.is_some_and(|max| context > max)
            {
                return None;
            }
            let request = ResourceRequest {
                ram_bytes: candidate
                    .estimated_resources
                    .ram_bytes?
                    .checked_add(overhead.ram_bytes)?,
                vram_bytes: candidate
                    .estimated_resources
                    .vram_bytes?
                    .checked_add(overhead.vram_bytes)?,
                context_tokens: context,
            };
            self.policy
                .budget
                .admit(&ResourceUsage::default(), &request)
                .ok()?;
            providers.push(
                self.registry
                    .get_provider(candidate.provider_id)
                    .await
                    .ok()?,
            );
        }
        let history = lock(&self.latency);
        let now = Instant::now();
        let mut best = None;
        for (candidate, provider) in candidates.iter().zip(providers) {
            let estimate = history.estimate(candidate, &provider, context, output_limit, now)?;
            let choice = (estimate, candidate.model_id);
            best = Some(best.map_or(choice, |previous| choice.min(previous)));
        }
        best.map(|(_, model_id)| model_id)
    }
}
