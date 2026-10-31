use modorch_config::*;
use modorch_core::*;
#[test]
fn parses_explicit_selection_and_rejects_bad_configuration() {
    let config = CompositionConfig::parse(
        r#"
[application]
name="orders"
[initiators.db]
enabled=true
[initiators.db.config]
url="local"
[modules.order]
enabled=true
[modules.worker]
enabled=false
[messaging]
default_queue_capacity=12
per_shard_capacity=3
publish_when_full="reject"
[shutdown]
grace_period_secs=2
"#,
    )
    .unwrap();
    let selection = config.selection().unwrap();
    assert_eq!(selection.components().count(), 2);
    assert_eq!(
        selection.config(&ComponentRef::initiator("db"))["url"],
        "local"
    );
    assert_eq!(config.grace_period().as_secs(), 2);
    for text in [
        "[application]\nname='a'\ninvalid=true",
        "[application]\nname='a'\n[messaging]\ndefault_queue_capacity=0",
        "[application]\nname='a'\n[messaging]\npublish_when_full='drop'",
        "[application]\nname='a'\n[modules.x]\nenabled='yes'",
    ] {
        assert!(CompositionConfig::parse(text).is_err());
    }
}
