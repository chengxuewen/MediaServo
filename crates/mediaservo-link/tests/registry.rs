//! Task 4: Registry registration/discovery/liveness tests (D235).

use mediaservo_link::{FrameTopic, NodeId, Registry, Role, registry::NodeInfo};

#[test]
fn register_then_discover_topics_and_nodes() {
    let info = NodeInfo {
        id: NodeId::new("capture-reg-0"),
        role: Role::Capture,
        publishes: vec!["camera/reg0/raw".into()],
        subscribes: vec![],
    };
    Registry::register(&info).unwrap();
    let topics = Registry::discover_topics("camera/reg0/").unwrap();
    assert!(topics.iter().any(|t| t.topic.as_str() == "camera/reg0/raw"));
    let nodes = Registry::discover_nodes(Role::Capture).unwrap();
    assert!(nodes.iter().any(|n| n.id.as_str() == "capture-reg-0"));
}

#[test]
fn mark_publisher_then_topic_publisher() {
    let id = NodeId::new("capture-pub-1");
    let topic = FrameTopic::new("camera/pub1/raw");
    Registry::mark_publisher(&topic, &id).unwrap();
    let pub_node = Registry::topic_publisher(&topic).unwrap();
    assert_eq!(pub_node.as_ref().map(|n| n.as_str()), Some("capture-pub-1"));
}

#[test]
fn topic_publisher_none_when_no_publisher() {
    let topic = FrameTopic::new("camera/none-topic/raw");
    assert!(Registry::topic_publisher(&topic).unwrap().is_none());
}

#[test]
fn unregister_removes_node_and_publishers() {
    let id = NodeId::new("capture-unreg-2");
    let topic = FrameTopic::new("camera/unreg2/raw");
    Registry::register(&NodeInfo {
        id: id.clone(),
        role: Role::Capture,
        publishes: vec![topic.as_str().to_string()],
        subscribes: vec![],
    })
    .unwrap();
    Registry::mark_publisher(&topic, &id).unwrap();
    assert_eq!(Registry::topic_publisher(&topic).unwrap().as_ref(), Some(&id));
    Registry::unregister(&id).unwrap();
    assert!(
        Registry::topic_publisher(&topic).unwrap().is_none(),
        "no active publisher after unregister"
    );
}
