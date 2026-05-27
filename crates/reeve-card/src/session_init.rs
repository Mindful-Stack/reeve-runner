//! SessionStart prompt template + render function for the planning agent.
//! Pure — no I/O, no infra deps. The handler in `handler.rs` calls
//! `render` with a card + resolved CardPaths and ships the result over
//! the wire to be injected into claude's session.

use crate::domain::Card;
use crate::transitions::CardPaths;

/// Render the SessionStart prompt for a card. The prompt is plugin-
/// agnostic: it names workflow phases (brainstorm, plan, execute,
/// test, review) and the household's installed plugin provides the
/// concrete skill bindings.
pub fn render(card: &Card, paths: &CardPaths) -> String {
    format!(
        "You are reeve card {id} in the {household} household.\n\
         Brief: {brief}\n\
         \n\
         Take this card through these phases:\n\
         \x20\x201. Brainstorm the design  \u{2192}  {spec_path}/{id}-design.md\n\
         \x20\x202. Write the implementation plan  \u{2192}  {plan_path}/{id}.md\n\
         \x20\x203. Execute the plan\n\
         \x20\x204. Run integration tests\n\
         \x20\x205. Submit for human review\n\
         \n\
         Your household's installed plugin provides the skills for each\n\
         phase \u{2014} invoke them as they apply.\n\
         \n\
         Signal phase changes to the kanban board as you progress:\n\
         \x20\x20reeve mv-self planning      (during brainstorming + planning)\n\
         \x20\x20reeve mv-self implementing  (when executing the plan)\n\
         \x20\x20reeve mv-self testing       (when running integration tests)\n\
         \x20\x20reeve mv-self review        (when ready for human review)\n\
         \n\
         The user may move your card to a different column to redirect you;\n\
         treat that as a phase-change instruction from them.",
        id = card.id(),
        household = card.household(),
        brief = card.brief(),
        spec_path = paths.spec_path,
        plan_path = paths.plan_path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_protocol::Column;

    fn card(id: &str, household: &str, brief: &str) -> Card {
        Card::from_persisted(
            id.into(),
            household.into(),
            brief.into(),
            Column::Planning,
            0,
            0,
            false,
            None,
        )
        .unwrap()
    }

    #[test]
    fn renders_with_default_paths() {
        let c = card("abc123", "witan", "Add OAuth login");
        let paths = CardPaths::from_optional(None, None);
        let out = render(&c, &paths);
        assert!(out.contains("reeve card abc123"));
        assert!(out.contains("in the witan household"));
        assert!(out.contains("Brief: Add OAuth login"));
        assert!(out.contains("docs/agent/specs/abc123-design.md"));
        assert!(out.contains("docs/agent/plans/abc123.md"));
        assert!(out.contains("reeve mv-self planning"));
        assert!(out.contains("reeve mv-self implementing"));
        assert!(out.contains("reeve mv-self testing"));
        assert!(out.contains("reeve mv-self review"));
    }

    #[test]
    fn renders_with_custom_paths() {
        let c = card("xyz", "ramudden", "Pipeline refactor");
        let paths = CardPaths::from_optional(
            Some("docs/superpowers/specs"),
            Some("docs/superpowers/plans"),
        );
        let out = render(&c, &paths);
        assert!(out.contains("docs/superpowers/specs/xyz-design.md"));
        assert!(out.contains("docs/superpowers/plans/xyz.md"));
        assert!(out.contains("in the ramudden household"));
    }
}
