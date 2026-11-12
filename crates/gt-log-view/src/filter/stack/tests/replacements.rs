use super::*;

#[rstest]
#[case::table(FilterEffect::Table, None)]
#[case::map(FilterEffect::Map, None)]
#[case::both(FilterEffect::Table, Some(FilterEffect::Map))]
fn replacements_preserve_condition_identity_effects_and_live_drafts(
    #[case] initial: FilterEffect,
    #[case] additional: Option<FilterEffect>,
    #[values(false, true)] enabled: bool,
) {
    let (mut stack, mut slots) = unfiltered_stack();
    let group = stack.create_group();
    stack.select_group(group);
    stack.set_live_filter_text("gnss");
    let id = match initial {
        FilterEffect::Table => stack.add_live_filter_as_chip().unwrap(),
        FilterEffect::Map => stack.add_live_filter_as_map_highlight(&mut slots).unwrap(),
    };
    if let Some(effect) = additional {
        stack.add_chip_effect(id, effect, &mut slots);
    }
    stack.set_chip_effect_enabled(id, initial, enabled);
    stack.wait_for_queries();
    stack.set_live_filter_effect(FilterEffect::Map);
    stack.set_live_filter_text("unapplied");
    let before = stack.chip(id).unwrap().to_stored_filter();
    let scan = stack.chip(id).unwrap().filter.query.scan_identity();
    let revision = stack.table_semantic_revision();
    let table = stack.chip(id).unwrap().is_enabled(FilterEffect::Table);
    let replacement =
        PreparedFilterCondition::try_from(&LiveFilterDraft::Service("hal-powerd".into())).unwrap();
    assert!(stack.replace_chip_condition(id, replacement));
    stack.wait_for_queries();
    let chip = stack.chip(id).unwrap();
    assert_eq!(chip.id(), id);
    assert_eq!(chip.group(), group);
    assert_eq!(chip.to_stored_filter().effects, before.effects);
    assert_eq!(chip.filter.query.scan_identity().0, scan.0);
    assert_eq!(
        chip.matches().matched_entry_indices().collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(stack.table_semantic_revision(), revision + u64::from(table));
    assert_eq!(stack.live_filter_text(), "unapplied");
    assert_eq!(stack.selected_chip(), Some(id));
    stack.remove_chip(id, &mut slots);
    assert_eq!(stack.selected_chip(), None);
}

#[test]
fn unchanged_and_removed_conditions_preserve_query_generations() {
    let (mut stack, mut slots) = unfiltered_stack();
    let id = add_refine_chip(&mut stack, "gnss");
    stack.set_live_filter_text("unapplied");
    let query = stack.chip(id).unwrap().filter.query.scan_identity();
    let revision = stack.table_semantic_revision();
    let unchanged =
        PreparedFilterCondition::try_from(&LiveFilterDraft::from(&FilterPattern::plain("gnss")))
            .unwrap();
    assert!(!stack.replace_chip_condition(id, unchanged));
    assert_eq!(stack.selected_chip(), Some(id));
    assert_eq!(stack.chip(id).unwrap().filter.query.scan_identity(), query);
    assert_eq!(stack.table_semantic_revision(), revision);
    stack.remove_chip(id, &mut slots);
    let missing =
        PreparedFilterCondition::try_from(&LiveFilterDraft::from(&FilterPattern::plain("battery")))
            .unwrap();
    assert!(!stack.replace_chip_condition(id, missing));
    assert_eq!(stack.selected_chip(), None);
}

#[rstest]
#[case::empty(LiveFilterDraft::Message { text: "  ".into(), regex: false })]
#[case::missing_level(LiveFilterDraft::Level(None))]
#[case::invalid_regex(LiveFilterDraft::Message { text: "[".into(), regex: true })]
fn incomplete_conditions_cannot_be_prepared(#[case] draft: LiveFilterDraft) {
    PreparedFilterCondition::try_from(&draft).unwrap_err();
}

#[rstest]
#[case::text(FilterPattern::plain("failed"), "failed")]
#[case::regex(FilterPattern::regex("f.il.d"), "failed")]
#[case::service(FilterPattern::Service("NAVSYNCD".into()), "navsyncd:")]
#[case::hostname(FilterPattern::Hostname("HOST".into()), "host")]
#[case::level(FilterPattern::Level(LogLevelKind::Error), "ERROR:")]
fn selected_conditions_highlight_matching_message_and_metadata_spans(
    #[case] pattern: FilterPattern,
    #[case] expected: &str,
) {
    let log = Arc::new(test_util::parsed_log_of_text(
        "2026-01-01 00:00:00 host navsyncd: ERROR: failed\n2026-01-01 00:00:01 other kernel: INFO: started\n",
    ));
    let mut stack = FilterStack::new(Arc::clone(&log));
    stack.set_live_draft(LiveFilterDraft::from(&pattern));
    let id = stack.add_live_filter_as_chip().unwrap();
    let message = log.message(log.entries().first().unwrap());
    assert_eq!(
        stack
            .condition_match_spans(0)
            .into_iter()
            .map(|span| message.get(span).unwrap())
            .collect::<Vec<_>>(),
        vec![expected]
    );
    assert_eq!(stack.condition_match_spans(1), Vec::<Range<usize>>::new());
    stack.clear_table_filters();
    assert!(stack.chip(id).is_none());
    assert_eq!(stack.selected_chip(), None);
}
