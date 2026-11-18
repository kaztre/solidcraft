//! How often SMLib's full check runs while a model is evaluated. Every sweep, boolean and blend
//! checks its result in SMLib before it is decoded ([`Validation::EachStep`]); an evaluation can
//! instead leave those checks to one check of the bodies each feature keeps
//! ([`Validation::EachFeature`]), or skip them ([`Validation::Off`]). A body kept straight from
//! a step is checked at the end as that step would have checked it, on the step's own result
//! (kept with the body until then): the record a result is decoded to can pass SMLib's check
//! where the result itself fails it (a surface of revolution's polar curve, rebuilt on import).
//!
//! The evaluation installs its mode around a feature with [`validating`], as [`crate::watch`]
//! installs a watch: the operations a feature runs are reached through hundreds of public kernel
//! functions, and the mode is the evaluation's, not each operation's. Outside [`validating`]
//! (the kernel called directly, a preview, a test) every step is checked, as it always was. The
//! worker threads the kernel starts for a step run under the same mode ([`crate::par`]).

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use solidcore_record::brep::Brep as Record;
use solidcraft_smlib::CheckLevel;

use crate::body::Body;
use crate::{KernelError, Result};

/// When SMLib's full check of a result runs during a model evaluation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Validation {
    /// After every sweep, boolean and blend: a step whose result fails the check fails the
    /// feature there (how every evaluation checked before the mode existed).
    EachStep,
    /// Once per feature: the steps inside a feature are not checked, and each body the feature
    /// makes or changes is checked before it is kept, so a feature that makes a body the check
    /// refuses still fails, with the check's reason.
    #[default]
    EachFeature,
    /// Never (experimental): a body the check would refuse is kept.
    Off,
}

impl Validation {
    pub const ALL: [Validation; 3] = [Validation::EachStep, Validation::EachFeature, Validation::Off];

    /// The mode's name: `each_step`, `each_feature` or `off` (as it is saved and sent).
    pub fn id(self) -> &'static str {
        match self {
            Validation::EachStep => "each_step",
            Validation::EachFeature => "each_feature",
            Validation::Off => "off",
        }
    }

    /// The mode named `id` ([`Validation::id`]), `None` for anything else.
    pub fn parse(id: &str) -> Option<Validation> {
        Validation::ALL.into_iter().find(|v| v.id() == id)
    }
}

/// One feature's work under a mode, shared with the worker threads it starts.
#[derive(Debug)]
pub(crate) struct Scope {
    mode: Validation,
    /// A step's check was left for the end of the feature ([`Validation::EachFeature`]).
    deferred: AtomicBool,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Scope>>> = const { RefCell::new(None) };
}

/// Run `f` with `mode` deciding which checks the kernel work it does on this thread runs. Also
/// gives whether a step's check was left for later under [`Validation::EachFeature`]: the
/// caller then checks the bodies it keeps ([`check_kept`]).
pub fn validating<R>(mode: Validation, f: impl FnOnce() -> R) -> (R, bool) {
    let scope = Arc::new(Scope { mode, deferred: AtomicBool::new(false) });
    let r = within(Some(scope.clone()), f);
    (r, scope.deferred.load(Ordering::Relaxed))
}

/// The scope this thread's work runs in, to hand to the worker threads it starts.
pub(crate) fn current() -> Option<Arc<Scope>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Run `f` in `scope` (one [`current`] gave on another thread), restoring this thread's own
/// afterwards, also when `f` panics.
pub(crate) fn within<R>(scope: Option<Arc<Scope>>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Option<Arc<Scope>>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                CURRENT.with(|c| *c.borrow_mut() = previous);
            }
        }
    }
    let previous = CURRENT.with(|c| c.replace(scope));
    let _restore = Restore(Some(previous));
    f()
}

/// What a step does with SMLib's check of its result ([`step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Check it now (outside [`validating`], and under [`Validation::EachStep`]).
    Check,
    /// Leave it to the end of the feature (under [`Validation::EachFeature`]): the body decoded
    /// from the result keeps it ([`Body::left_unchecked`]) for [`check_kept`].
    Defer,
    /// Never check it (under [`Validation::Off`]).
    Skip,
}

/// What the step about to check its result does. [`Step::Defer`] also tells the feature a check
/// was left for its end.
pub(crate) fn step() -> Step {
    CURRENT.with(|c| match c.borrow().as_deref() {
        None | Some(Scope { mode: Validation::EachStep, .. }) => Step::Check,
        Some(s @ Scope { mode: Validation::EachFeature, .. }) => {
            s.deferred.store(true, Ordering::Relaxed);
            Step::Defer
        }
        Some(Scope { mode: Validation::Off, .. }) => Step::Skip,
    })
}

/// How a step's own check takes its result.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Checked {
    /// As it is.
    AsIs,
    /// A boolean's: solids touching each other along an edge or at a vertex come back as one
    /// non-manifold body, checked as the separate solids its export holds (the boolean's rule).
    Combined,
}

/// A step's SMLib result whose check was left to the end of its feature: kept with the body
/// decoded from it (and only for that body's record) until [`check_kept`] takes it, so the check
/// at the end is the step's own, of the result the step made.
#[derive(Debug)]
pub(crate) struct Unchecked {
    record: std::sync::Weak<solidcore_record::brep::Brep>,
    /// The step, as its own refusal names it (`SMLib fillet`).
    what: String,
    how: Checked,
    /// The result, with the records of the bodies the step combined (to name one that already
    /// fails the check): let go together once the check takes them.
    result: std::sync::Mutex<Option<(solidcraft_smlib::Brep, Vec<Arc<Record>>)>>,
}

impl Unchecked {
    pub(crate) fn new(record: &Arc<Record>, what: &str, how: Checked, result: solidcraft_smlib::Brep, operands: &[&Body]) -> Unchecked {
        let operands = operands.iter().map(|b| b.record.arc().clone()).collect();
        Unchecked { record: Arc::downgrade(record), what: what.to_string(), how, result: std::sync::Mutex::new(Some((result, operands))) }
    }

    /// The step's result and its operands, taken (once), when this is `record`'s.
    fn take(&self, record: &Arc<solidcore_record::brep::Brep>) -> Option<(solidcraft_smlib::Brep, Vec<Arc<Record>>)> {
        if !std::ptr::eq(self.record.as_ptr(), Arc::as_ptr(record)) {
            return None;
        }
        self.result.lock().ok().and_then(|mut r| r.take())
    }
}

/// SMLib's full check of `r` as a step takes it (`how`): the check's report.
pub(crate) fn check_result(r: &solidcraft_smlib::Brep, how: Checked) -> std::result::Result<solidcraft_smlib::CheckReport, String> {
    let check = match separate(r, how)? {
        Some(record) => solidcraft_smlib::Brep::from_record(&record, true).map_err(|e| format!("the separate solids: {e}"))?.check(CheckLevel::All),
        None => r.check(CheckLevel::All),
    };
    check.map_err(|e| format!("check: {e}"))
}

/// The record of the separate solids a boolean's result `r` exports to where it is not one
/// manifold solid ([`Checked::Combined`]), which its check takes; `None`: `r` is checked as it is.
fn separate(r: &solidcraft_smlib::Brep, how: Checked) -> std::result::Result<Option<Record>, String> {
    let split = matches!(how, Checked::Combined) && !r.is_manifold_solid().map_err(|e| format!("check: {e}"))?;
    split.then(|| r.to_native().map_err(|e| format!("export: {e}"))).transpose()
}

/// What a step says of a result SMLib's check refuses, naming the operands (the input of a
/// blend) whose own check (`checks`, in order) fails too: the fault is then theirs.
pub(crate) fn refuses(checks: impl ExactSizeIterator<Item = solidcraft_smlib::Result<solidcraft_smlib::CheckReport>>) -> String {
    let n = checks.len();
    let name = |i: usize| match n {
        1 => "the input".to_string(),
        2 => ["the first operand", "the second operand"][i.min(1)].to_string(),
        _ => format!("operand {i}"),
    };
    let failing: Vec<String> = checks.enumerate().filter(|(_, c)| c.as_ref().is_ok_and(|c| !c.is_valid())).map(|(i, _)| name(i)).collect();
    let whose = if failing.is_empty() { String::new() } else { format!(" ({} already fails it)", failing.join(" and ")) };
    format!("SMLib's check refuses the result{whose}")
}

/// A body SMLib's check of what a feature keeps refuses ([`check_kept_counted`]).
#[derive(Debug)]
pub struct KeptRefusal {
    /// How many tests of the check failed; `None` when the check itself could not run.
    pub failures: Option<usize>,
    /// The refusal, as [`check_kept`] gives it.
    pub error: KernelError,
}

/// SMLib's full check of a body a feature keeps, once a step of the feature left its check to
/// the end: an error with the check's reason unless it passes. A body decoded from a step's
/// result is checked as that step would have checked it, on its result; any other (a copy moved
/// into place, pieces joined without an operation) as SMLib takes it for the operations after it.
/// A mesh body has nothing to check.
pub fn check_kept(body: &Body) -> Result<()> {
    check_kept_counted(body).map_err(|r| r.error)
}

/// [`check_kept`], with how many tests a refused body fails: a feature that changes a body the
/// check already refused (a part imported with defects) can tell whether it made the body worse.
pub fn check_kept_counted(body: &Body) -> std::result::Result<(), KeptRefusal> {
    if body.is_mesh() {
        return Ok(());
    }
    // A body checked before is not checked again: the check of a heavy imported body takes a
    // minute, and a feature on a body that already fails it asked for it up to three times a
    // step (the body before, its result beside the features after, its result again in line).
    if let Some(known) = known::get(body.record.arc()) {
        return known;
    }
    let verdict = match body.unchecked.as_deref().and_then(|u| u.take(body.record.arc()).map(|r| (u, r))) {
        Some((u, (r, operands))) => verdict_counted(&u.what, check_result_owned(r, u.how), &operands),
        None => match body.smlib() {
            Ok(s) => verdict_counted("SMLib", s.check(CheckLevel::All).map_err(|e| format!("check: {e}")), &[]),
            Err(error) => return Err(KeptRefusal { failures: None, error }),
        },
    };
    known::put(body.record.arc(), &verdict);
    verdict
}

/// Whether [`check_kept_counted`] already refused `body` (nothing is checked here): a feature
/// about to change such a body checks its result in line, where the result may be kept, not
/// beside the features after it, where a refusal means doing the feature again.
pub fn known_refused(body: &Body) -> bool {
    matches!(known::get(body.record.arc()), Some(Err(_)))
}

/// The verdicts [`check_kept_counted`] has reached, by the body's record (bodies share a record
/// when one is a copy of the other): kept only while the record lives.
mod known {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, Weak};

    use solidcore_record::brep::Brep;

    use super::KeptRefusal;
    use crate::KernelError;

    /// A verdict as it is kept: passed, or the failed tests and the refusal's text.
    type Kept = std::result::Result<(), (Option<usize>, String)>;

    static KNOWN: Mutex<Option<HashMap<usize, (Weak<Brep>, Kept)>>> = Mutex::new(None);
    /// Verdicts kept at most; past it the ones whose bodies are gone are dropped.
    const MOST: usize = 4096;

    pub(super) fn get(of: &Arc<Brep>) -> Option<std::result::Result<(), KeptRefusal>> {
        let known = KNOWN.lock().ok()?;
        let (record, kept) = known.as_ref()?.get(&(Arc::as_ptr(of) as usize))?;
        // The address is the record's only while that record lives.
        if !record.upgrade().is_some_and(|r| Arc::ptr_eq(&r, of)) {
            return None;
        }
        Some(match kept {
            Ok(()) => Ok(()),
            Err((failures, text)) => Err(KeptRefusal { failures: *failures, error: KernelError::Failed(text.clone()) }),
        })
    }

    pub(super) fn put(of: &Arc<Brep>, verdict: &std::result::Result<(), KeptRefusal>) {
        // Only a verdict the check reached: one it could not run for (stopped, out of memory) is
        // asked for again.
        let kept: Kept = match verdict {
            Ok(()) => Ok(()),
            Err(KeptRefusal { failures: Some(n), error: KernelError::Failed(text) }) => Err((Some(*n), text.clone())),
            Err(_) => return,
        };
        let Ok(mut known) = KNOWN.lock() else { return };
        let known = known.get_or_insert_with(HashMap::new);
        if known.len() >= MOST {
            known.retain(|_, (record, _)| record.strong_count() > 0);
            if known.len() >= MOST {
                known.clear();
            }
        }
        known.insert(Arc::as_ptr(of) as usize, (Arc::downgrade(of), kept));
    }
}

/// What [`check_kept_counted`] makes of SMLib's check of what step `what` made of `operands`,
/// with how many tests a refused result fails.
fn verdict_counted(
    what: &str,
    check: std::result::Result<solidcraft_smlib::CheckReport, String>,
    operands: &[Arc<Record>],
) -> std::result::Result<(), KeptRefusal> {
    match check {
        Ok(c) if c.is_valid() => Ok(()),
        Ok(c) => {
            let checks = operands.iter().map(|o| solidcraft_smlib::Brep::from_record(o, true).and_then(|s| s.check_owned(CheckLevel::All)));
            Err(KeptRefusal { failures: Some(c.failures), error: KernelError::Failed(format!("{what}: {}: {c}", refuses(checks))) })
        }
        Err(e) => Err(KeptRefusal { failures: None, error: KernelError::Failed(format!("{what}: {e}")) }),
    }
}

/// [`check_kept`] of a body a feature made from `before` (the body it changed, when there was
/// one). A body the check refuses is still kept when `before` was refused already and the
/// feature left it failing no more tests than it came with (a part imported with defects, cut
/// or joined): the feature is not what the check objects to, and refusing it left such a part
/// impossible to change at all (kaztre#15). `Some((was, now))` then gives the failed tests
/// before and after, for the feature's warning; `None` is a body the check accepts.
/// (`now` is `None` for a body kept without a check, [`kept_as_refused`].)
pub fn check_kept_from(body: &Body, before: Option<&Body>) -> Result<Option<(usize, Option<usize>)>> {
    let Err(refused) = check_kept_counted(body) else { return Ok(None) };
    let came_with = before.and_then(|b| check_kept_counted(b).err()).and_then(|r| r.failures);
    match (came_with, refused.failures) {
        (Some(was), Some(now)) if now <= was => Ok(Some((was, Some(now)))),
        _ => Err(refused.error),
    }
}

/// Keeps `body`, made from `before`, without a check when the check is already known to refuse
/// `before`: the failed tests `before` came with, `None` when its verdict is not known or it
/// passed (the body is then checked as any other). The check of a heavy part takes seconds,
/// every feature on a defective part paid for it, and its result was kept either way unless it
/// failed more tests than the part came with. The body takes the verdict of the one it was made
/// from, so the features after it are not checked either. Not for a feature that mends a body.
pub fn kept_as_refused(body: &Body, before: &Body) -> Option<usize> {
    if body.is_mesh() {
        return None;
    }
    let came = known::get(before.record.arc())?.err()?;
    let was = came.failures?;
    if let Some(own) = known::get(body.record.arc()) {
        return own.is_err().then_some(was);
    }
    // The step's own result is not needed for a check: dropped now, not with the body.
    drop(body.unchecked.as_deref().and_then(|u| u.take(body.record.arc())));
    known::put(body.record.arc(), &Err(came));
    Some(was)
}

/// [`check_kept_counted`] of a body by its record alone, as SMLib takes it for an operation: for
/// the body a detached check's body was made from ([`KeptCheck::run_from`]), on that check's
/// thread, reading nothing the evaluation uses.
fn check_record(record: &Arc<solidcore_record::brep::Brep>) -> std::result::Result<(), KeptRefusal> {
    if let Some(known) = known::get(record) {
        return known;
    }
    let check = solidcraft_smlib::Brep::from_record(record, true)
        .map_err(|e| format!("{e}"))
        .and_then(|b| b.check_owned(CheckLevel::All).map_err(|e| format!("check: {e}")));
    let verdict = verdict_counted("SMLib", check, &[]);
    known::put(record, &verdict);
    verdict
}

/// SMLib's check of a body as an operation on it takes it ([`Body::smlib`], at every test): what
/// it fails, or `None` when it passes or is not SMLib's to make (a mesh body, one SMLib refuses
/// outright: [`Body::smlib_refusal`] says why). A check that cannot run is the error why.
///
/// The check runs on a copy of the body's SMLib form, which it owns, so the token over this
/// thread's work stops it ([`solidcraft_smlib::Brep::check_owned`]), as it stops a feature's
/// detached check ([`KeptCheck`]): an import stopped part way stops in the check too.
pub fn failing_check(body: &Body) -> Result<Option<CheckFailure>> {
    if body.is_mesh() || body.smlib_refusal().is_some() {
        return Ok(None);
    }
    let copy = body.smlib()?.try_clone().map_err(|e| KernelError::smlib("SMLib copy", e))?;
    let check = copy.check_owned(solidcraft_smlib::CheckLevel::All).map_err(|e| KernelError::smlib("SMLib's check", e))?;
    Ok(CheckFailure::of(&check))
}

/// What SMLib's check failed in a body: how many tests failed, and each test that failed, where
/// the check reports it (its site: the test's number and name, and the kind of entity it failed
/// on), with how many times. Shown in a line ([`std::fmt::Display`]): `3 failed tests: SmEdge::Test
/// #7 Bad Edge Length: less than edge tolerance (×2); SmLoop::Test #2 Bad Loop Orient`, in the
/// order the check first reported them. Each failure's own values stay in the check's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckFailure {
    /// Failed tests, as the check counts them.
    pub failures: usize,
    /// The tests that failed, each once, in the order the check first reported them.
    pub tests: Vec<FailedTest>,
}

/// One test SMLib's check failed: its site in the check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedTest {
    /// The test as the check names it (`SmEdge::Test #7 Bad Edge Length: less than edge
    /// tolerance`), without the values of each failure.
    pub name: String,
    /// The test's number (`7`), when the check gives one.
    pub number: Option<u32>,
    /// The kinds of entity it failed on (`SmVertex`), each once, when the check names them.
    pub owners: Vec<String>,
    /// How many times it failed.
    pub times: usize,
}

impl CheckFailure {
    /// The failure `check` reports, `None` when it passes.
    pub fn of(check: &solidcraft_smlib::CheckReport) -> Option<CheckFailure> {
        if check.is_valid() {
            return None;
        }
        let mut tests: Vec<FailedTest> = Vec::new();
        // "AssertValid skipped" says why the second half of the check did not run: not a test failed.
        for line in check.text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("AssertValid skipped")) {
            // The test's name ends where its values or its owner start.
            let name =
                [line.find(" ("), line.find(" [owner")].into_iter().flatten().min().map_or(line, |end| line.get(..end).unwrap_or(line)).trim_end();
            let owner = line.rsplit_once("[owner ").and_then(|(_, rest)| rest.strip_suffix(']')).map(str::trim).filter(|o| !o.is_empty());
            let i = match tests.iter().position(|t| t.name == name) {
                Some(i) => i,
                None => {
                    let number = name.split_once("Test #").and_then(|(_, rest)| {
                        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                        digits.parse().ok()
                    });
                    tests.push(FailedTest { name: name.to_string(), number, owners: Vec::new(), times: 0 });
                    tests.len() - 1
                }
            };
            if let Some(t) = tests.get_mut(i) {
                t.times += 1;
                if let Some(owner) = owner.filter(|o| !t.owners.iter().any(|known| known == o)) {
                    t.owners.push(owner.to_string());
                }
            }
        }
        Some(CheckFailure { failures: check.failures, tests })
    }
}

impl std::fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = if self.failures == 1 { "" } else { "s" };
        write!(f, "{} failed test{s}: ", self.failures)?;
        for (i, t) in self.tests.iter().enumerate() {
            let sep = if i == 0 { "" } else { "; " };
            if t.times > 1 { write!(f, "{sep}{} (×{})", t.name, t.times)? } else { write!(f, "{sep}{}", t.name)? }
        }
        Ok(())
    }
}

/// [`check_result`] of a step's result nobody else reads, which it takes: the token over this
/// thread's work stops it ([`solidcraft_smlib::Brep::check_owned`]).
fn check_result_owned(r: solidcraft_smlib::Brep, how: Checked) -> std::result::Result<solidcraft_smlib::CheckReport, String> {
    let check = match separate(&r, how)? {
        Some(record) => {
            drop(r);
            solidcraft_smlib::Brep::from_record(&record, true).map_err(|e| format!("the separate solids: {e}"))?.check_owned(CheckLevel::All)
        }
        None => r.check_owned(CheckLevel::All),
    };
    check.map_err(|e| format!("check: {e}"))
}

/// [`check_kept`] of a body, ready to run on another thread ([`detach_kept`]), under the token
/// of the work that detached it: stopping that work stops the check too.
pub struct KeptCheck(Kept, Option<solidcraft_smlib::stop::Stop>, Made);

/// The body a [`KeptCheck`] was detached from, and the one a feature made it from: by their
/// records, which the check's thread may read while the evaluation goes on.
struct Made {
    record: std::sync::Weak<solidcore_record::brep::Brep>,
    before: Option<Arc<solidcore_record::brep::Brep>>,
}

enum Kept {
    /// A mesh body: nothing to check.
    Nothing,
    /// The step's own result, taken from the body.
    Step { what: String, how: Checked, result: solidcraft_smlib::Brep, operands: Vec<Arc<Record>> },
    /// A copy of the body's SMLib form, made when the check was detached.
    Copy(Result<solidcraft_smlib::Brep>),
}

/// [`check_kept`] of `body`, detached from it: the step's result is taken now, or the body's SMLib
/// form is copied now, so the check reads nothing the operations after it use and can run on
/// another thread beside them ([`KeptCheck::run`]).
pub fn detach_kept(body: &Body) -> KeptCheck {
    detach_kept_from(body, None)
}

/// [`detach_kept`] of a body a feature made from `before` (the body it changed, when there was
/// one): the detached check then keeps a refused body as [`check_kept_from`] does
/// ([`KeptCheck::run_from`]), on its own thread, where a refusal used to cost the feature a
/// second evaluation and two more checks in line.
pub fn detach_kept_from(body: &Body, before: Option<&Body>) -> KeptCheck {
    let stop = solidcraft_smlib::stop::current();
    let made = Made { record: Arc::downgrade(body.record.arc()), before: before.filter(|b| !b.is_mesh()).map(|b| b.record.arc().clone()) };
    if body.is_mesh() {
        return KeptCheck(Kept::Nothing, stop, made);
    }
    if let Some((u, (result, operands))) = body.unchecked.as_deref().and_then(|u| u.take(body.record.arc()).map(|r| (u, r))) {
        return KeptCheck(Kept::Step { what: u.what.clone(), how: u.how, result, operands }, stop, made);
    }
    KeptCheck(Kept::Copy(body.smlib().and_then(|b| b.try_clone().map_err(|e| KernelError::smlib("SMLib copy", e)))), stop, made)
}

impl KeptCheck {
    /// The check: [`check_kept`]'s result for the body it was detached from.
    pub fn run(self) -> Result<()> {
        self.run_from().map(|_| ())
    }

    /// The check: [`check_kept_from`]'s result for the body it was detached from and the one
    /// that body was made from. The verdict is kept for the body ([`known_refused`]), so the
    /// feature after this one need not ask for it again.
    pub fn run_from(self) -> Result<Option<(usize, Option<usize>)>> {
        let KeptCheck(kept, stop, made) = self;
        solidcraft_smlib::stop::scoped(stop, || {
            let verdict = kept.run();
            if let Some(record) = made.record.upgrade() {
                known::put(&record, &verdict);
            }
            let Err(refused) = verdict else { return Ok(None) };
            let came_with = made.before.as_ref().and_then(|b| check_record(b).err()).and_then(|r| r.failures);
            match (came_with, refused.failures) {
                (Some(was), Some(now)) if now <= was => Ok(Some((was, Some(now)))),
                _ => Err(refused.error),
            }
        })
    }
}

impl Kept {
    fn run(self) -> std::result::Result<(), KeptRefusal> {
        match self {
            Kept::Nothing => Ok(()),
            // The step's result and the copy are the check's own: the work's token stops it.
            Kept::Step { what, how, result, operands } => verdict_counted(&what, check_result_owned(result, how), &operands),
            Kept::Copy(Ok(copy)) => verdict_counted("SMLib", copy.check_owned(CheckLevel::All).map_err(|e| format!("check: {e}")), &[]),
            Kept::Copy(Err(error)) => Err(KeptRefusal { failures: None, error }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_checked_outside_an_evaluation_and_under_each_step() {
        assert_eq!(step(), Step::Check);
        assert_eq!(validating(Validation::EachStep, step), (Step::Check, false));
        assert_eq!(validating(Validation::EachFeature, step), (Step::Defer, true));
        assert_eq!(validating(Validation::Off, step), (Step::Skip, false));
        // The thread's own mode comes back, also after a nested scope.
        let ((inner, outer), deferred) = validating(Validation::Off, || (validating(Validation::EachStep, step).0, step()));
        assert!(inner == Step::Check && outer == Step::Skip && !deferred);
        assert_eq!(step(), Step::Check);
    }

    /// A check detached under a token runs under it on whatever thread runs it (the feature
    /// check beside the evaluation): stopping the work stops the check (solidcore#3247).
    #[test]
    fn a_detached_check_stops_with_the_work_that_detached_it() {
        let body = crate::cylinder(solidcore_record::Vec3::new(0.0, 0.0, 0.0), solidcore_record::Vec3::Z, 3.0, 10.0).unwrap();
        let stop = solidcraft_smlib::stop::Stop::new();
        let detached = stop.within(|| detach_kept(&body));
        stop.stop();
        let r = crate::par::kernel_thread("check").spawn(move || detached.run()).unwrap().join().unwrap();
        assert!(r.as_ref().is_err_and(|e| e.to_string().contains("cancelled")), "{r:?}");
        let free = solidcraft_smlib::stop::Stop::new();
        let detached = free.within(|| detach_kept(&body));
        assert!(detached.run().is_ok(), "a token never stopped changes nothing");
    }

    /// #17689: [`failing_check`] checks a copy it owns, so a stopped token stops it (an import
    /// stopped part way), and it names each test the check failed, at its site, once.
    #[test]
    fn a_failing_check_stops_with_the_work_and_names_each_failed_test() {
        use solidcore_record::Vec3;
        let good = crate::box_solid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 10.0)).unwrap();
        let mut record = good.record().clone();
        record.vertices[0].point[2] += 0.5;
        let bad = Body::from_record(record).unwrap();
        let stop = solidcraft_smlib::stop::Stop::new();
        stop.stop();
        let r = stop.within(|| failing_check(&bad));
        assert!(r.as_ref().is_err_and(|e| e.to_string().contains("cancelled")), "{r:?}");
        // Run again with no token: the body's own SMLib form was not touched by the stopped check.
        let failure = failing_check(&bad).unwrap().expect("the moved corner fails the check");
        let report = bad.smlib().unwrap().check(solidcraft_smlib::CheckLevel::All).unwrap();
        assert_eq!(failure.failures, report.failures);
        let lines: Vec<&str> = report.text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("AssertValid skipped")).collect();
        assert_eq!(failure.tests.iter().map(|t| t.times).sum::<usize>(), lines.len(), "{}", report.text);
        for t in &failure.tests {
            assert_eq!(lines.iter().filter(|l| l.starts_with(&t.name)).count(), t.times, "{t:?}");
            assert_eq!(
                t.number.map(|n| n.to_string()),
                t.name.split_once("Test #").map(|(_, r)| r.chars().take_while(char::is_ascii_digit).collect())
            );
            assert!(t.owners.iter().all(|o| lines.iter().any(|l| l.starts_with(&t.name) && l.ends_with(&format!("[owner {o}]")))), "{t:?}");
        }
        assert!(failure.to_string().starts_with(&format!("{} failed test", report.failures)), "{failure}");
        assert_eq!(failing_check(&good).unwrap(), None, "the box passes");
    }

    #[test]
    fn the_mode_reaches_the_kernels_worker_threads() {
        let items = [0, 1, 2, 3];
        let (on_threads, deferred) = validating(Validation::EachFeature, || crate::par::map_on_threads(&items, 4, |_| step()));
        assert_eq!(on_threads, vec![Some(Step::Defer); 4]);
        assert!(deferred);
    }

    /// A box with one corner moved off its three edges: SMLib's check refuses it. Under each_step
    /// the boolean that takes it refuses its result there; under each_feature it is not checked,
    /// says the check was left for later, and the check of the body kept refuses it. Both name
    /// the operand that already fails the check (#17230), also when the check runs detached.
    #[test]
    fn a_body_the_check_refuses_is_refused_at_the_step_or_at_the_end() {
        use solidcore_record::Vec3;
        let good = crate::box_solid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 10.0)).unwrap();
        assert!(check_kept(&good).is_ok());
        let mut record = good.record().clone();
        record.vertices[0].point[2] += 0.5;
        let bad = Body::from_record(record).unwrap();
        let e = check_kept(&bad).unwrap_err().to_string();
        assert!(e.contains("SMLib's check refuses the result"), "{e}");
        let tool = crate::box_solid(Vec3::new(5.0, 5.0, 5.0), Vec3::new(20.0, 20.0, 20.0)).unwrap();
        let (step, deferred) = validating(Validation::EachStep, || crate::boolean(&bad, &tool, crate::BoolOp::Union));
        let e = step.unwrap_err().to_string();
        assert!(e.contains("SMLib's check refuses the result (the first operand already fails it)") && !deferred, "{e}");
        let (joined, deferred) = validating(Validation::EachFeature, || crate::boolean(&bad, &tool, crate::BoolOp::Union));
        assert!(deferred);
        let joined = joined.unwrap().unwrap();
        let detached = detach_kept(&joined).run().unwrap_err().to_string();
        assert!(detached.contains("SMLib boolean: SMLib's check refuses the result (the first operand already fails it)"), "{detached}");
        let (joined, _) = validating(Validation::EachFeature, || crate::boolean(&bad, &tool, crate::BoolOp::Union));
        let e = check_kept(&joined.unwrap().unwrap()).unwrap_err().to_string();
        assert!(e.contains("SMLib boolean: SMLib's check refuses the result (the first operand already fails it)"), "{e}");
        // Only the operand that fails is named.
        let (joined, _) = validating(Validation::EachFeature, || crate::boolean(&tool, &bad, crate::BoolOp::Union));
        let e = check_kept(&joined.unwrap().unwrap()).unwrap_err().to_string();
        assert!(e.contains("(the second operand already fails it)"), "{e}");
        // The checks took the operands' records with the results: the bodies kept don't hold them.
        assert_eq!(Arc::strong_count(bad.record.arc()), 1);
        // A round on an edge away from the moved corner: the blend names its input the same way.
        let edge = [Vec3::new(10.0, 5.0, 10.0)];
        let (step, _) = validating(Validation::EachStep, || crate::fillet(&bad, &edge, 1.0));
        let e = step.unwrap_err().to_string();
        assert!(e.contains("SMLib's check refuses the result (the input already fails it)"), "{e}");
        let (round, deferred) = validating(Validation::EachFeature, || crate::fillet(&bad, &edge, 1.0));
        let e = check_kept(&round.unwrap()).unwrap_err().to_string();
        assert!(deferred && e.contains("SMLib fillet: SMLib's check refuses the result (the input already fails it)"), "{e}");
        let (off, deferred) = validating(Validation::Off, || crate::boolean(&bad, &tool, crate::BoolOp::Union));
        assert!(off.is_ok_and(|b| b.is_some()) && !deferred);
    }

    /// A body the check refused before a feature is kept when the feature made it no worse: the
    /// same defective box moved is kept (with its failures before and after), while a sound box
    /// turned into it is refused as before, and so is a refused body with nothing before it.
    #[test]
    fn a_body_no_worse_than_it_came_is_kept() {
        use solidcore_record::Vec3;
        let good = crate::box_solid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 10.0)).unwrap();
        let mut record = good.record().clone();
        record.vertices[0].point[2] += 0.5;
        let bad = Body::from_record(record).unwrap();
        let moved = crate::transform_matrix(&bad, [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [5.0, 0.0, 0.0, 1.0]]).unwrap();
        let failures = check_kept_counted(&bad).unwrap_err().failures.unwrap();
        assert!(failures > 0);
        // Detached, the check keeps it the same way, on the records alone, and its verdict stays.
        let fresh = Body::from_record(bad.record().clone()).unwrap();
        let again =
            crate::transform_matrix(&fresh, [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [7.0, 0.0, 0.0, 1.0]]).unwrap();
        assert_eq!(detach_kept_from(&again, Some(&fresh)).run_from().unwrap(), Some((failures, Some(failures))));
        assert!(known_refused(&again) && known_refused(&fresh));
        assert!(detach_kept_from(&again, None).run_from().is_err());
        assert!(detach_kept_from(&again, Some(&good)).run_from().is_err());
        // Made from a body known to be refused, it is kept without a check of its own.
        assert_eq!(kept_as_refused(&moved, &bad), Some(failures));
        assert!(known_refused(&moved) && kept_as_refused(&moved, &good).is_none());
        assert_eq!(check_kept_from(&moved, Some(&bad)).unwrap(), Some((failures, Some(failures))));
        assert_eq!(check_kept_from(&good, Some(&bad)).unwrap(), None);
        let e = check_kept_from(&moved, Some(&good)).unwrap_err().to_string();
        assert!(e.contains("SMLib's check refuses the result"), "{e}");
        assert!(check_kept_from(&moved, None).is_err());
    }

    #[test]
    fn modes_are_named_as_they_are_saved() {
        for v in Validation::ALL {
            assert_eq!(Validation::parse(v.id()), Some(v));
            assert_eq!(serde_json::to_value(v).ok(), Some(serde_json::json!(v.id())));
        }
        assert_eq!(Validation::parse("each step"), None);
        assert_eq!(Validation::default(), Validation::EachFeature);
    }
}
