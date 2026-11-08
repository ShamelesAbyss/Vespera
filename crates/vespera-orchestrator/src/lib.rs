//! Bounded, authorized sequential workflows over actual independent agent execution.
//! Plans are operator inputs. Generated content cannot change routing or permissions.
mod decomposition;
mod plan;
mod runtime;
mod storage;
pub use decomposition::{DecompositionError, DecompositionRequest, WorkerCapabilities};
pub use plan::{ConflictPolicy, OutputPolicy, PlanError, Step, Workflow};
pub use runtime::{Attempt, Failure, Orchestrator, OrchestratorError, WorkflowReport};
pub use storage::{StoredWorkflow, WorkflowStore, WorkflowStoreError};
