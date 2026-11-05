use serde_json::json;
use vespera_core::{DomainError, Task, TaskState};

#[test]
fn complete_transition_matrix_rejects_self_and_terminal_transitions() {
    let states = [
        TaskState::Queued,
        TaskState::Running,
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Cancelled,
    ];
    for from in states {
        for to in states {
            let permitted = matches!(
                (from, to),
                (TaskState::Queued, TaskState::Running | TaskState::Cancelled)
                    | (
                        TaskState::Running,
                        TaskState::Completed | TaskState::Failed | TaskState::Cancelled
                    )
            );
            let mut state = from;
            let result = state.transition(to);
            if permitted {
                assert_eq!(result, Ok(()), "{from} -> {to}");
                assert_eq!(state, to);
            } else {
                assert_eq!(result, Err(DomainError::InvalidTaskTransition { from, to }));
                assert_eq!(state, from, "rejected transition mutated task");
            }
        }
    }
}

#[test]
fn task_snapshots_are_validated_and_default_to_queued() {
    let mut task: Task = serde_json::from_value(json!({
        "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "prompt": "Summarize the supplied text"
    }))
    .unwrap();
    task.validate().unwrap();
    assert_eq!(task.state, TaskState::Queued);
    assert!(task.required_capabilities.is_empty());
    task.state.transition(TaskState::Running).unwrap();
    task.state.transition(TaskState::Completed).unwrap();
    assert_eq!(
        serde_json::from_value::<Task>(serde_json::to_value(&task).unwrap()).unwrap(),
        task
    );
    task.prompt = "\n\t ".into();
    assert!(task.validate().is_err());
}
