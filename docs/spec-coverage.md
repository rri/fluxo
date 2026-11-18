# Spec coverage

Which tests check each rule in `docs/spec.md`. Rust tests are `file::test` under
`crates/fluxo-web/tests/` (unit tests: `crates/fluxo-web/src/<module>.rs`); browser tests are named
by their title in `tests/browser/flows.spec.mjs` and run in Chromium (with and without JavaScript)
and Firefox. Keep this table in step with the spec: a rule with no test here is a gap.

| Rule | Tests |
|---|---|
| G1 | spec::g1_security_headers · browser "pages send a strict content security policy…" |
| G2 | the whole browser suite in the no-JavaScript project |
| G3 | spec::g3_every_action_needs_the_form_token |
| G4 | spec::g4_g7_not_found_reveals_nothing · launch::unknown_pages_get_a_proper_404 |
| G5 | spec::g5_server_errors_say_nothing_more |
| G6 | spec::g6_health_says_nothing_else · launch::health_check_answers_even_before_setup |
| G7 | spec::g4_g7_not_found_reveals_nothing · signin::static_files_are_served |
| G8 | spec::g8_v2_s3_what_each_kind_of_person_sees · collaboration::invite_accept_and_contribute (badge) |
| G9 | accounts::settings_save_and_validate · spec::g9_g16_… · browser "theme is saved to the account" |
| G10 | security::g10_user_text_is_escaped_everywhere_it_shows |
| G11 | browser "keyboard shortcuts go to real pages (G11)" |
| G12 | spec::g8_v2_s3_what_each_kind_of_person_sees |
| G13 | spec::g13_control_and_bidi_characters_are_removed_from_text |
| G14 | spec::g14_picker_labels_show_at_most_two_goals_above |
| G15 | browser "every page meets WCAG 2.1 AA in light and dark (G15)" |
| G16 | spec::g9_g16_pages_are_light_and_sign_in_uses_the_system_theme |
| G17 | browser "every page fits a phone screen (G17)" |
| G18 | spec::g18_every_page_ends_with_the_source_footer |
| PF1 | spec::pf1_a_very_deep_tree_stays_cheap_for_everyone · access::deep_trees_take_one_pass · access::one_pass_access_matches_the_rules |
| PF2 | spec::pf2_malformed_requests_are_refused_without_errors · spec::go6_a_malformed_parent_creates_nothing |
| F1–F3, F5 | spec::f1_f2_a_fresh_instance_needs_the_setup_link · accounts::first_person_sets_up_the_instance_as_administrator |
| F4 | accounts::first_person_sets_up_… · spec::f4_only_one_first_account_even_when_two_race |
| V1 | spec::v1_signed_out_visitors_are_sent_to_sign_in · spec::v1_signed_out_pages_link_back_to_sign_in_the_same_way |
| V2 | spec::g8_v2_s3_… · signin::password_then_code_then_focus |
| V3 | signin::password_then_code_then_focus (same answer either way) · spec::v3_a_wrong_password_keeps_the_username. The equal timing is by construction (a dummy hash for unknown names), not timed in tests. |
| V4 | security::v4_* · security::changing_the_case_of_a_username_does_not_dodge_the_limit · launch::repeated_wrong_passwords_are_slowed_down · throttle::addresses_are_bucketed |
| V5 | signin::password_then_code_then_focus · signin::without_2fa_password_signs_straight_in |
| V6 | security::v6_session_ids_change_at_sign_in_and_old_ones_die |
| V7 | accounts::sign_up_follows_the_administrators_choice · username::* · spec::d4_a_deleted_account_keeps_its_username |
| V8 | accounts::sign_up_follows_the_administrators_choice · browser "an invitation link brings in one new person" |
| V9 | accounts::administrator_code_gets_a_locked_out_person_back_in · spec::v9_… · browser "a locked-out person gets back in…" |
| V10 | signin::password_then_code_then_focus |
| H1 | signin::password_then_code_then_focus (replay) · spec::h1_codes_from_the_neighbouring_steps_work_and_older_ones_do_not |
| H2 | accounts::turning_on_2fa_gives_recovery_codes_that_work_once · browser "a recovery code signs in once" |
| H3 | signin::five_wrong_codes_end_the_attempt |
| H4 | signin::password_then_code_then_focus · spec::se1_idle_sessions_are_purged |
| R1, R2 | accounts::administrator_code_gets_a_locked_out_person_back_in |
| FO1, FO2 | spec::fo1_fo2_fo3_focus_sections_in_order_and_updates_age_out · collaboration::invite_accept_and_contribute |
| FO3 | spec::go2_fo3_fo4_empty_states · spec::fo1_fo2_fo3_… · collaboration::invite_accept_and_contribute |
| FO3a | collaboration::watchers_hear_about_outlook_changes_only · focus::daily_check_reports_dates_slipping |
| FO4 | focus::* · spec::go2_fo3_fo4_empty_states · browser "Focus ranks next steps and can start one" |
| FO5 | spec::fo5_gp1_gp4_t1_titles_of_private_goals_never_leak |
| GO1 | spec::go1_your_goals_come_first · goals::break_a_goal_down_into_a_path · forecasts::goal_page_shows_dates_and_the_critical_path |
| GO2 | spec::go2_fo3_fo4_empty_states · goals::break_a_goal_down_into_a_path |
| GO3 | goals::links_refuse_loops_and_keep_tasks_attached |
| GO4 | goals::archive_restore_and_purge · access::archived_items_are_for_owners_only |
| GO5 | goals::access_follows_ownership_roles_and_privacy |
| GO6 | spec::go6_a_malformed_parent_creates_nothing |
| GP1 | spec::fo5_gp1_gp4_t1_… |
| GP2 | goals::break_a_goal_down_into_a_path · goals::task_progress_and_achieving_the_goal · spec::gp2_gp3_… |
| GP3 | forecasts::* · spec::gp2_gp3_the_stamp_and_whose_calendar_dates_use · goals::risks_keep_their_likelihood_history (top risks) |
| GP4 | forecasts::goal_page_shows_dates_and_the_critical_path · goals::break_a_goal_down_into_a_path · spec::fo5_… · browser "a goal shows its forecast" (tracing) |
| GP5 | forecasts::empty_goals_beneath_are_flagged_until_sized_or_broken_down · spec::go2_fo3_fo4_empty_states |
| GP6 | goals::access_follows_ownership_roles_and_privacy |
| GP7 | goals::task_progress_and_achieving_the_goal · spec::gp7_only_archive_sits_in_the_red_box · collaboration::ownership_transfer_needs_acceptance |
| GP8, GP10 | goals::archive_restore_and_purge · access::archived_items_are_for_owners_only |
| GP9 | collaboration::invite_accept_and_contribute · collaboration::watchers_hear_… · collaboration::private_goals_are_not_watchable_… |
| GP11 | spec::gp2_gp3_the_stamp_and_whose_calendar_dates_use |
| TP1 | forecasts::goal_page_shows_dates_and_the_critical_path · spec::fo5_… (breadcrumbs) |
| TP2 | goals::task_progress_and_achieving_the_goal · goals::access_follows_ownership_roles_and_privacy |
| TP3 | spec::tp3_l1_l5_connections_and_their_limits |
| TP4 | goals::links_refuse_loops_and_keep_tasks_attached |
| TP5 | goals::archive_restore_and_purge · goals::access_follows_ownership_roles_and_privacy |
| L1 | goals::links_refuse_loops_and_keep_tasks_attached · spec::tp3_l1_l5_… · access::goals_cannot_contain_themselves |
| L2 | spec::l2_contributors_cannot_take_or_expose_work_by_relinking · access::relinking_must_not_change_control |
| L3 | goals::links_refuse_loops_and_keep_tasks_attached |
| L4 | goals::links_refuse_loops_and_keep_tasks_attached |
| L5 | goals::risks_keep_their_likelihood_history · spec::tp3_l1_l5_… |
| A1, A2 | goals::archive_restore_and_purge · access::archived_items_are_for_owners_only |
| S1 | accounts::settings_save_and_validate · accounts::turning_on_2fa_gives_recovery_codes_that_work_once |
| S1a | accounts::turning_on_2fa_gives_recovery_codes_that_work_once · spec::s1a_2fa_setup_offers_a_way_out |
| S2 | admin::there_is_always_an_administrator · spec::s2_notices_come_from_a_fixed_list |
| S2a | accounts::sign_up_follows_the_administrators_choice · accounts::administrator_code_… · accounts::turning_on_2fa_… |
| S3 | spec::g8_v2_s3_… · accounts::sign_up_follows_the_administrators_choice |
| S4 | accounts::* · admin::* · goals::archive_restore_and_purge (retention) |
| S5 | launch::export_holds_what_you_can_see |
| D1 | admin::deleting_an_account_hands_work_over |
| D2 | admin::deleting_an_account_hands_work_over · spec::d2_t1_a_declined_transfer_leaves_the_owner_in_place |
| D3 | spec::d3_i1_handover_replaces_earlier_offers_and_archived_invitations_wait |
| D4 | admin::deleting_an_account_hands_work_over · spec::d4_a_deleted_account_keeps_its_username |
| T1 | collaboration::ownership_transfer_needs_acceptance · spec::fo5_… · spec::d2_t1_… |
| I1 | collaboration::invite_accept_and_contribute · collaboration::declined_and_expired_invitations · spec::d3_i1_… |
| SE1 | spec::se1_idle_sessions_are_purged |
| SE2 | security::se2_cookie_flags |
