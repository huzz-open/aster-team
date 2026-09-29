use super::*;

#[tokio::test]
async fn abandoned_task_sends_cancel_only_to_its_original_connection() {
    let hub = RunnerHub::default();
    let (_, mut original) = hub.register("runner-a").await;
    let (_, cancellation) = hub.dispatch("runner-a", "task-a", vec![1]).await.unwrap();
    assert_eq!(original.recv().await.unwrap(), vec![1]);
    drop(cancellation);
    assert!(
        matches!(decode_control_frame_for_test(&original.recv().await.unwrap()), ControlToRunner::CancelTask(frame) if frame.task_id=="task-a")
    );
    let (_, cancelled_after_reconnect) = hub.dispatch("runner-a", "task-b", vec![2]).await.unwrap();
    original.recv().await.unwrap();
    let (_, mut replacement) = hub.register("runner-a").await;
    drop(cancelled_after_reconnect);
    assert!(original.recv().await.is_none());
    assert!(replacement.try_recv().is_err());
}

fn decode_control_frame_for_test(bytes: &[u8]) -> ControlToRunner {
    aster_runner_protocol::decode_control_frame(bytes).unwrap()
}

#[tokio::test]
async fn terminal_task_is_not_cancelled_again_when_result_is_consumed() {
    let hub = RunnerHub::default();
    let (_, mut wire) = hub.register("runner-a").await;
    let (mut events, cancellation) = hub.dispatch("runner-a", "task-a", vec![1]).await.unwrap();
    wire.recv().await.unwrap();
    hub.publish(RunnerToControl::TaskFinished(
        aster_runner_protocol::TaskResult {
            task_id: "task-a".into(),
            status: 200,
            usage_json: None,
        },
    ))
    .await;
    assert!(matches!(
        events.recv().await.unwrap(),
        RunnerToControl::TaskFinished(_)
    ));
    drop(cancellation);
    assert!(wire.try_recv().is_err());
}

#[tokio::test]
async fn disconnect_closes_only_tasks_from_the_departed_connection_generation() {
    let hub = RunnerHub::default();
    let (old_generation, mut old_wire) = hub.register("runner-a").await;
    let (mut old_events, _old_cancel) =
        hub.dispatch("runner-a", "old-task", vec![1]).await.unwrap();
    old_wire.recv().await.unwrap();
    let (new_generation, mut new_wire) = hub.register("runner-a").await;
    let (mut new_events, _new_cancel) =
        hub.dispatch("runner-a", "new-task", vec![2]).await.unwrap();
    new_wire.recv().await.unwrap();
    hub.unregister("runner-a", old_generation).await;
    assert!(old_events.recv().await.is_none());
    assert!(matches!(
        new_events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    hub.unregister("runner-a", new_generation).await;
    assert!(new_events.recv().await.is_none());
}
