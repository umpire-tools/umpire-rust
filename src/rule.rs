use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::checkops::CheckOp;
use crate::error::Result;
use crate::evaluator::{Ctx, State};
use crate::expr::{CompiledExpr, Pred};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Constraint {
    Enabled,
    Fair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Requires,
    EnabledWhen,
    FairWhen,
    Disables,
    OneOf,
    AnyOf,
    EitherOf,
}

impl Kind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Kind::Requires => "requires",
            Kind::EnabledWhen => "enabledWhen",
            Kind::FairWhen => "fairWhen",
            Kind::Disables => "disables",
            Kind::OneOf => "oneOf",
            Kind::AnyOf => "anyOf",
            Kind::EitherOf => "eitherOf",
        }
    }
}

/// One requires-dependency: either a field name or a compiled expression,
/// kept in declaration order.
#[derive(Clone, Debug)]
pub(crate) enum Dep {
    Field(String),
    Expr(CompiledExpr),
}

#[derive(Clone, Debug)]
pub(crate) struct NamedFields {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct NamedRules {
    // Carried from the schema for diagnostics; evaluation reads the rules.
    #[allow(dead_code)]
    pub name: String,
    pub rules: Vec<Rule>,
}

/// How a fairWhen rule tests its field's value.
#[derive(Clone, Debug)]
pub(crate) enum FairTest {
    Expr(CompiledExpr),
    Check(CheckOp),
}

/// A single availability or fairness rule targeting one or more fields.
/// Construct rules with the exported constructors or by parsing a JSON
/// schema.
#[derive(Clone, Debug)]
pub struct Rule {
    // Unique per rule; keys the oneOf evaluation cache.
    pub(crate) id: usize,
    pub(crate) kind: Kind,
    pub(crate) targets: Vec<String>,
    pub(crate) sources: Vec<String>,
    pub(crate) constraint: Constraint,
    pub(crate) reason: Option<String>,

    // requires
    pub(crate) deps: Vec<Dep>,

    // enabledWhen / disables(when)
    pub(crate) when: Option<CompiledExpr>,

    // fairWhen
    pub(crate) fair: Option<FairTest>,

    // disables(source)
    pub(crate) disable_source: Option<String>,

    // oneOf
    pub(crate) group: String,
    pub(crate) branches: Vec<NamedFields>,

    // composites
    pub(crate) inner: Vec<Rule>,
    pub(crate) branch_rules: Vec<NamedRules>,
}

fn next_rule_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn base_rule(kind: Kind, targets: Vec<String>, reason: Option<String>) -> Rule {
    Rule {
        id: next_rule_id(),
        kind,
        targets,
        sources: Vec::new(),
        constraint: Constraint::Enabled,
        reason,
        deps: Vec::new(),
        when: None,
        fair: None,
        disable_source: None,
        group: String::new(),
        branches: Vec::new(),
        inner: Vec::new(),
        branch_rules: Vec::new(),
    }
}

/// The result of evaluating one rule for one target field. `fair` is
/// tri-state: `None` means the rule does not speak to fairness (a pass).
#[derive(Clone, Debug, PartialEq)]
pub struct RuleEvaluation {
    pub enabled: bool,
    pub fair: Option<bool>,
    pub reason: String,
    pub reasons: Vec<String>,
}

impl RuleEvaluation {
    pub(crate) const fn pass() -> RuleEvaluation {
        RuleEvaluation {
            enabled: true,
            fair: None,
            reason: String::new(),
            reasons: Vec::new(),
        }
    }
}

fn pass_eval() -> RuleEvaluation {
    RuleEvaluation::pass()
}

fn disabled_eval(reason: String) -> RuleEvaluation {
    RuleEvaluation {
        enabled: false,
        fair: None,
        reason,
        reasons: Vec::new(),
    }
}

fn bool_some(b: bool) -> Option<bool> {
    Some(b)
}

impl Rule {
    pub(crate) fn reason_or<'a>(&'a self, fallback: &'a str) -> &'a str {
        match &self.reason {
            Some(reason) => reason.as_str(),
            None => fallback,
        }
    }

    // graphSources splits the rule's source fields into ordering edges (which
    // drive topological ordering) and informational edges. Mirrors the TS
    // core's getGraphSourceInfo.
    pub(crate) fn graph_sources(&self) -> (Vec<String>, Vec<String>) {
        match self.kind {
            Kind::EnabledWhen | Kind::FairWhen => {
                if let Some(when) = &self.when {
                    if !when.check_field.is_empty() {
                        return (Vec::new(), vec![when.check_field.clone()]);
                    }
                }
                (self.sources.clone(), Vec::new())
            }
            Kind::AnyOf | Kind::EitherOf => {
                let inner_rules: Vec<&Rule> = if self.kind == Kind::AnyOf {
                    self.inner.iter().collect()
                } else {
                    self.branch_rules
                        .iter()
                        .flat_map(|b| b.rules.iter())
                        .collect()
                };
                let mut ordering_set: Vec<String> = Vec::new();
                for inner in &inner_rules {
                    let (ordering, _) = inner.graph_sources();
                    for f in ordering {
                        if !ordering_set.contains(&f) {
                            ordering_set.push(f);
                        }
                    }
                }
                ordering_set.sort();
                let mut informational = Vec::new();
                for inner in &inner_rules {
                    let (_, info) = inner.graph_sources();
                    for f in info {
                        if !ordering_set.contains(&f) && !informational.contains(&f) {
                            informational.push(f);
                        }
                    }
                }
                (ordering_set, informational)
            }
            _ => (self.sources.clone(), Vec::new()),
        }
    }

    // evaluateTarget evaluates the rule for one target field. Composites and
    // oneOf route through evaluate_rule instead.
    pub(crate) fn evaluate_target(
        &self,
        field: &str,
        ctx: &Ctx,
        state: &mut State,
    ) -> Result<RuleEvaluation> {
        match self.kind {
            Kind::Requires => self.evaluate_requires(ctx, state),
            Kind::EnabledWhen => {
                let when = self.when.as_ref().expect("enabledWhen carries a when expr");
                let passed = when.eval(ctx.values, ctx.conditions)?;
                if passed {
                    return Ok(pass_eval());
                }
                Ok(disabled_eval(
                    self.reason_or("condition not met").to_string(),
                ))
            }
            Kind::FairWhen => {
                let value = ctx.values.get(field).unwrap_or(&Value::Null);
                let def = ctx.fields.get(field);
                let satisfied = def.is_some_and(|d| crate::field::is_satisfied(value, d));
                if !satisfied {
                    return Ok(RuleEvaluation {
                        enabled: true,
                        fair: bool_some(true),
                        reason: String::new(),
                        reasons: Vec::new(),
                    });
                }
                let fair = self.fair.as_ref().expect("fairWhen carries a fair test");
                let passed = match fair {
                    FairTest::Expr(expr) => expr.eval(ctx.values, ctx.conditions)?,
                    FairTest::Check(op) => !value.is_null() && op.validate(value),
                };
                if passed {
                    return Ok(RuleEvaluation {
                        enabled: true,
                        fair: bool_some(true),
                        reason: String::new(),
                        reasons: Vec::new(),
                    });
                }
                Ok(RuleEvaluation {
                    enabled: true,
                    fair: bool_some(false),
                    reason: self.reason_or("selection is no longer valid").to_string(),
                    reasons: Vec::new(),
                })
            }
            Kind::Disables => {
                let active = if let Some(source) = &self.disable_source {
                    let value = ctx.values.get(source).unwrap_or(&Value::Null);
                    let def = ctx.fields.get(source);
                    def.is_some_and(|d| crate::field::is_satisfied(value, d))
                } else {
                    let when = self.when.as_ref().expect("disables carries a when expr");
                    when.eval(ctx.values, ctx.conditions)?
                };
                if !active {
                    return Ok(pass_eval());
                }
                let label = self.disable_source.as_deref().unwrap_or("condition");
                Ok(disabled_eval(
                    self.reason_or(&format!("overridden by {}", label))
                        .to_string(),
                ))
            }
            _ => Ok(pass_eval()),
        }
    }

    fn evaluate_requires(&self, ctx: &Ctx, state: &State) -> Result<RuleEvaluation> {
        let mut reason = String::new();
        let mut reasons: Vec<String> = Vec::new();

        for dep in &self.deps {
            let satisfied;
            let fallback;
            match dep {
                Dep::Field(name) => {
                    fallback = format!("requires {}", name);
                    let value = ctx.values.get(name).unwrap_or(&Value::Null);
                    let def = ctx.fields.get(name);
                    let dep_satisfied = def.is_some_and(|d| crate::field::is_satisfied(value, d));
                    let dep_avail = state.availability.get(name);
                    let dep_enabled = dep_avail.is_none_or(|a| a.enabled);
                    let dep_fair = dep_avail.is_none_or(|a| a.fair);
                    satisfied = dep_satisfied && dep_enabled && dep_fair;
                }
                Dep::Expr(expr) => {
                    fallback = "required condition not met".to_string();
                    satisfied = expr.eval(ctx.values, ctx.conditions)?;
                }
            }

            if satisfied {
                continue;
            }

            let resolved = self.reason_or(&fallback).to_string();
            if reason.is_empty() {
                reason = resolved.clone();
            }
            reasons.push(resolved);
        }

        if reasons.is_empty() {
            return Ok(pass_eval());
        }
        Ok(RuleEvaluation {
            enabled: false,
            fair: None,
            reason,
            reasons,
        })
    }

    // evaluateOneOf resolves the oneOf branch and returns per-target results.
    // Mirrors the TS core's resolveOneOfState.
    fn evaluate_one_of(
        &self,
        ctx: &Ctx,
    ) -> Result<std::collections::HashMap<String, RuleEvaluation>> {
        let satisfied_branches: Vec<&str> = self
            .branches
            .iter()
            .filter(|branch| {
                branch.fields.iter().any(|f| {
                    let value = ctx.values.get(f).unwrap_or(&Value::Null);
                    ctx.fields
                        .get(f)
                        .is_some_and(|d| crate::field::is_satisfied(value, d))
                })
            })
            .map(|b| b.name.as_str())
            .collect();

        let mut active = String::new();
        if satisfied_branches.len() == 1 {
            active = satisfied_branches[0].to_string();
        } else if satisfied_branches.len() > 1 {
            if let Some(prev) = ctx.prev {
                let newly: Vec<&str> = satisfied_branches
                    .iter()
                    .copied()
                    .filter(|name| {
                        !self.branches.iter().any(|branch| {
                            branch.name == *name
                                && branch.fields.iter().any(|f| {
                                    let value = prev.get(f).unwrap_or(&Value::Null);
                                    ctx.fields
                                        .get(f)
                                        .is_some_and(|d| crate::field::is_satisfied(value, d))
                                })
                        })
                    })
                    .collect();
                if newly.len() == 1 {
                    active = newly[0].to_string();
                }
            }
            if active.is_empty() {
                // Ambiguous fallback: first satisfied branch.
                active = satisfied_branches[0].to_string();
            }
        }

        let mut result = std::collections::HashMap::new();
        for branch in &self.branches {
            for target in &branch.fields {
                if active.is_empty() {
                    result.insert(target.clone(), pass_eval());
                    continue;
                }
                let in_active = self
                    .branches
                    .iter()
                    .any(|b| b.name == active && b.fields.iter().any(|f| f == target));
                if in_active {
                    result.insert(target.clone(), pass_eval());
                } else {
                    result.insert(
                        target.clone(),
                        disabled_eval(
                            self.reason_or(&format!("conflicts with {} strategy", active))
                                .to_string(),
                        ),
                    );
                }
            }
        }
        Ok(result)
    }
}

// evaluateRule evaluates a rule for one target field, recursing through
// composites. Mirrors the TS evaluator's evaluateRuleForField /
// evaluateRuleForTarget pair.
pub(crate) fn evaluate_rule(
    rule: &Rule,
    field: &str,
    ctx: &Ctx,
    state: &mut State,
) -> Result<RuleEvaluation> {
    match rule.kind {
        Kind::AnyOf => evaluate_any_of(rule, field, ctx, state),
        Kind::EitherOf => evaluate_either_of(rule, field, ctx, state),
        Kind::OneOf => {
            if let Some(cached) = state.cache.get(&rule.id) {
                if let Some(result) = cached.get(field) {
                    return Ok(result.clone());
                }
                return Ok(pass_eval());
            }
            let results = rule.evaluate_one_of(ctx)?;
            state.cache.insert(rule.id, results.clone());
            if let Some(result) = results.get(field) {
                return Ok(result.clone());
            }
            Ok(pass_eval())
        }
        _ => rule.evaluate_target(field, ctx, state),
    }
}

fn evaluate_any_of(
    rule: &Rule,
    field: &str,
    ctx: &Ctx,
    state: &mut State,
) -> Result<RuleEvaluation> {
    let mut passed = false;
    let mut reasons: Vec<String> = Vec::new();

    for inner in &rule.inner {
        let result = evaluate_rule(inner, field, ctx, state)?;
        if composite_passed(rule.constraint, &result) {
            passed = true;
            reasons = Vec::new();
            break;
        }
        append_composite_failure_reasons(&result, &mut reasons);
    }

    if passed {
        return Ok(composite_pass_result(rule.constraint));
    }
    Ok(composite_failure_result(rule.constraint, reasons))
}

fn evaluate_either_of(
    rule: &Rule,
    field: &str,
    ctx: &Ctx,
    state: &mut State,
) -> Result<RuleEvaluation> {
    let mut branch_results = Vec::with_capacity(rule.branch_rules.len());

    for branch in &rule.branch_rules {
        let mut inner_results = Vec::with_capacity(branch.rules.len());
        for inner in &branch.rules {
            let result = evaluate_rule(inner, field, ctx, state)?;
            inner_results.push(result);
        }
        branch_results.push(combine_composite_results(
            rule.constraint,
            Mode::And,
            inner_results,
        ));
    }

    Ok(combine_composite_results(
        rule.constraint,
        Mode::Or,
        branch_results,
    ))
}

enum Mode {
    And,
    Or,
}

fn composite_passed(constraint: Constraint, result: &RuleEvaluation) -> bool {
    if constraint == Constraint::Fair {
        return result.fair.is_none_or(|f| f);
    }
    result.enabled
}

fn composite_pass_result(constraint: Constraint) -> RuleEvaluation {
    if constraint == Constraint::Fair {
        return RuleEvaluation {
            enabled: true,
            fair: bool_some(true),
            reason: String::new(),
            reasons: Vec::new(),
        };
    }
    pass_eval()
}

fn composite_failure_result(constraint: Constraint, reasons: Vec<String>) -> RuleEvaluation {
    let mut reason = String::new();
    if let Some(first) = reasons.first() {
        reason = first.clone();
    }
    if constraint == Constraint::Fair {
        return RuleEvaluation {
            enabled: true,
            fair: bool_some(false),
            reason,
            reasons,
        };
    }
    RuleEvaluation {
        enabled: false,
        fair: None,
        reason,
        reasons,
    }
}

fn append_composite_failure_reasons(result: &RuleEvaluation, reasons: &mut Vec<String>) {
    if !result.reasons.is_empty() {
        reasons.extend(result.reasons.iter().cloned());
        return;
    }
    if !result.reason.is_empty() {
        reasons.push(result.reason.clone());
    }
}

fn combine_composite_results(
    constraint: Constraint,
    mode: Mode,
    results: Vec<RuleEvaluation>,
) -> RuleEvaluation {
    let mut passed = matches!(mode, Mode::And);

    for result in &results {
        let current_passed = composite_passed(constraint, result);
        match mode {
            Mode::And => {
                if !current_passed {
                    passed = false;
                    break;
                }
            }
            Mode::Or => {
                if current_passed {
                    passed = true;
                    break;
                }
            }
        }
    }

    if passed {
        return composite_pass_result(constraint);
    }

    let mut reasons = Vec::new();
    for result in &results {
        append_composite_failure_reasons(result, &mut reasons);
    }
    composite_failure_result(constraint, reasons)
}

// Programmatic constructors -----------------------------------------------

impl Rule {
    /// Gates a field on a predicate.
    pub fn enabled_when(field: impl Into<String>, when: Pred, reason: &str) -> Rule {
        let mut rule = base_rule(Kind::EnabledWhen, vec![field.into()], non_empty(reason));
        rule.when = Some(CompiledExpr {
            pred: when,
            check_field: String::new(),
        });
        rule
    }

    /// Marks a field unfair when its predicate fails.
    pub fn fair_when(field: impl Into<String>, pred: Pred, reason: &str) -> Rule {
        let mut rule = base_rule(Kind::FairWhen, vec![field.into()], non_empty(reason));
        rule.constraint = Constraint::Fair;
        rule.fair = Some(FairTest::Expr(CompiledExpr {
            pred,
            check_field: String::new(),
        }));
        rule
    }

    /// Makes a field required-until-satisfied against another field.
    pub fn requires_dependency(
        field: impl Into<String>,
        dependency: impl Into<String>,
        reason: &str,
    ) -> Rule {
        let dependency = dependency.into();
        let mut rule = base_rule(Kind::Requires, vec![field.into()], non_empty(reason));
        rule.sources = vec![dependency.clone()];
        rule.deps = vec![Dep::Field(dependency)];
        rule
    }

    /// Gates a field's requirement on a predicate.
    pub fn requires_predicate(field: impl Into<String>, when: Pred, reason: &str) -> Rule {
        let mut rule = base_rule(Kind::Requires, vec![field.into()], non_empty(reason));
        rule.deps = vec![Dep::Expr(CompiledExpr {
            pred: when,
            check_field: String::new(),
        })];
        rule
    }

    /// Disables targets while a source field holds a satisfied value.
    pub fn disables_field(source: impl Into<String>, targets: Vec<String>, reason: &str) -> Rule {
        let source_str = source.into();
        let mut rule = base_rule(Kind::Disables, targets, non_empty(reason));
        rule.sources = vec![source_str.clone()];
        rule.disable_source = Some(source_str);
        rule
    }

    /// Disables targets while a predicate holds.
    pub fn disables_when(when: Pred, targets: Vec<String>, reason: &str) -> Rule {
        let mut rule = base_rule(Kind::Disables, targets, non_empty(reason));
        rule.when = Some(CompiledExpr {
            pred: when,
            check_field: String::new(),
        });
        rule
    }

    /// Allows only one branch of fields to be enabled at a time. The active
    /// branch is detected from satisfied values, with prev-assisted
    /// resolution.
    ///
    /// # Panics
    /// When a field appears in multiple branches.
    pub fn one_of(
        group: impl Into<String>,
        branches: Vec<(String, Vec<String>)>,
        reason: &str,
    ) -> Rule {
        let group = group.into();
        let named: Vec<NamedFields> = branches
            .into_iter()
            .map(|(name, fields)| NamedFields { name, fields })
            .collect();
        let mut targets: Vec<String> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for branch in &named {
            for f in &branch.fields {
                if seen.contains(f) {
                    panic!(
                        "[@umpire/core] oneOf({:?}) field {:?} appears in multiple branches",
                        group, f
                    );
                }
                seen.push(f.clone());
                targets.push(f.clone());
            }
        }
        let mut rule = base_rule(Kind::OneOf, targets.clone(), non_empty(reason));
        rule.sources = targets;
        rule.group = group;
        rule.branches = named;
        rule
    }

    /// Passes when any inner rule passes.
    ///
    /// # Panics
    /// When the rules are empty or target different fields.
    pub fn any_of(rules: Vec<Rule>) -> Rule {
        assert!(
            !rules.is_empty(),
            "[@umpire/core] anyOf() requires at least one rule"
        );
        let constraint = rules[0].constraint;
        let targets = rules[0].targets.clone();
        for r in &rules[1..] {
            if r.constraint != constraint || !same_targets(&r.targets, &targets) {
                panic!("[@umpire/core] anyOf() rules must target the same fields");
            }
        }
        let mut sources: Vec<String> = Vec::new();
        for r in &rules {
            for s in &r.sources {
                if !sources.contains(s) {
                    sources.push(s.clone());
                }
            }
        }
        let mut rule = base_rule(Kind::AnyOf, targets, None);
        rule.sources = sources;
        rule.constraint = constraint;
        rule.inner = rules;
        rule
    }

    /// Passes when any branch's ANDed rules all pass.
    ///
    /// # Panics
    /// When the branches are empty or the rules target different fields.
    pub fn either_of(group: impl Into<String>, branches: Vec<(String, Vec<Rule>)>) -> Rule {
        let group = group.into();
        assert!(
            !branches.is_empty(),
            "[@umpire/core] eitherOf({:?}) must include at least one branch",
            group
        );
        let mut all: Vec<&Rule> = Vec::new();
        for (name, rules) in &branches {
            assert!(
                !rules.is_empty(),
                "[@umpire/core] eitherOf({:?}) branch {:?} must not be empty",
                group,
                name
            );
            all.extend(rules.iter());
        }
        let constraint = all[0].constraint;
        let targets = all[0].targets.clone();
        for r in &all[1..] {
            if r.constraint != constraint || !same_targets(&r.targets, &targets) {
                panic!(
                    "[@umpire/core] eitherOf({:?}) rules must target the same fields",
                    group
                );
            }
        }
        let mut rule = base_rule(Kind::EitherOf, targets, None);
        rule.constraint = constraint;
        rule.group = group;
        rule.branch_rules = branches
            .into_iter()
            .map(|(name, rules)| NamedRules { name, rules })
            .collect();
        rule
    }

    /// Builds a field-bound named check as a fairness rule.
    pub fn check(field: impl Into<String>, op: CheckOp, reason: &str) -> Rule {
        let resolved = if !reason.is_empty() {
            reason.to_string()
        } else {
            let op_reason = op.reason();
            if op_reason.is_empty() {
                "Invalid value".to_string()
            } else {
                op_reason
            }
        };
        let field = field.into();
        let mut rule = base_rule(Kind::FairWhen, vec![field.clone()], Some(resolved));
        rule.sources = vec![field];
        rule.constraint = Constraint::Fair;
        rule.fair = Some(FairTest::Check(op));
        rule
    }
}

fn non_empty(reason: &str) -> Option<String> {
    if reason.is_empty() {
        None
    } else {
        Some(reason.to_string())
    }
}

fn same_targets(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    b.iter().all(|f| a.contains(f))
}
