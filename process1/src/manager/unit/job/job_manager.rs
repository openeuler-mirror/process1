#![warn(unused_imports)]
use super::job_alloc::JobAlloc;
use super::job_entry::{self, Job, JobConf, JobInfo, JobKind, JobResult};
use super::job_notify::{self};
use super::job_stat::JobStat;
use super::job_table::JobTable;
use super::job_transaction::{self};
use super::JobErrno;
use crate::manager::data::{UnitActiveState, UnitNotifyFlags};
use crate::manager::table::{TableOp, TableSubscribe};
use crate::manager::unit::unit_base::{JobMode, UnitRelationAtom};
use crate::manager::unit::unit_datastore::UnitDb;
use crate::manager::unit::unit_entry::UnitX;
use event::{EventState, EventType, Events, Source};
use std::cell::RefCell;
use std::rc::Rc;
use utils::{Error, Result};

#[derive(Debug)]
pub(in crate::manager::unit) struct JobAffect {
    // data
    pub(in crate::manager::unit) adds: Vec<JobInfo>,
    pub(in crate::manager::unit) dels: Vec<JobInfo>,
    pub(in crate::manager::unit) updates: Vec<JobInfo>,

    // control
    interested: bool,
}

impl JobAffect {
    pub(in crate::manager::unit) fn new(interested: bool) -> JobAffect {
        JobAffect {
            adds: Vec::new(),
            dels: Vec::new(),
            updates: Vec::new(),

            interested,
        }
    }

    fn record(&mut self, jobs: &(Vec<Rc<Job>>, Vec<Rc<Job>>, Vec<Rc<Job>>)) {
        if self.interested {
            let (adds, dels, updates) = jobs;
            self.adds.append(&mut jobs_2_jobinfo(adds));
            self.dels.append(&mut jobs_2_jobinfo(dels));
            self.updates.append(&mut jobs_2_jobinfo(updates));
        }
    }
}

pub(in crate::manager::unit) struct JobManager {
    // associated objects
    event: Rc<Events>,

    // owned objects
    sub_name: String, // key for table-subscriber: UnitSets
    data: Rc<JobManagerData>,
}

impl JobManager {
    pub(in crate::manager::unit) fn new(dbr: &Rc<UnitDb>, eventr: &Rc<Events>) -> JobManager {
        let jm = JobManager {
            event: Rc::clone(eventr),
            sub_name: String::from("JobManager"),
            data: Rc::new(JobManagerData::new(dbr)),
        };
        jm.register(eventr, dbr);
        jm
    }

    pub(in crate::manager::unit) fn exec(
        &self,
        config: &JobConf,
        mode: JobMode,
        affect: &mut JobAffect,
    ) -> Result<(), JobErrno> {
        self.data.exec(config, mode, affect)?;
        self.try_enable();
        Ok(())
    }

    pub(in crate::manager::unit) fn notify(
        &self,
        config: &JobConf,
        mode: JobMode,
    ) -> Result<(), JobErrno> {
        self.data.notify(config, mode)?;
        self.try_enable();
        Ok(())
    }

    pub(in crate::manager::unit) fn try_finish(
        &self,
        unit: &Rc<UnitX>,
        os: UnitActiveState,
        ns: UnitActiveState,
        flags: UnitNotifyFlags,
    ) -> Result<(), JobErrno> {
        self.data.try_finish(unit, os, ns, flags)?;
        self.try_enable();
        Ok(())
    }

    pub(in crate::manager::unit) fn remove(&self, id: u32) -> Result<(), JobErrno> {
        self.data.remove(id)?;
        self.try_enable();
        Ok(())
    }

    pub(in crate::manager::unit) fn get_jobinfo(&self, id: u32) -> Option<JobInfo> {
        self.data.get_jobinfo(id)
    }

    pub(in crate::manager::unit) fn has_stop_job(&self, unit: &Rc<UnitX>) -> bool {
        match self.data.get_suspends(unit) {
            Some(_) => true,
            None => false,
        }
    }

    fn try_enable(&self) {
        // prepare for async-running
        if self.data.is_jobs_ready() {
            self.enable(&self.event);
        }
    }

    fn register(&self, eventr: &Rc<Events>, dbr: &Rc<UnitDb>) {
        // event
        let source = Rc::clone(&self.data);
        eventr.add_source(source).unwrap();

        // db
        let subscriber = Rc::clone(&self.data);
        dbr.units_register(&self.sub_name, subscriber);
    }

    fn enable(&self, eventr: &Rc<Events>) {
        let source = Rc::clone(&self.data);
        eventr.set_enabled(source, EventState::OneShot).unwrap();
    }
}

impl Source for JobManagerData {
    fn event_type(&self) -> EventType {
        EventType::Defer
    }

    fn epoll_event(&self) -> u32 {
        0
    }

    fn priority(&self) -> i8 {
        -10
    }

    fn token(&self) -> u64 {
        let data: u64 = unsafe { std::mem::transmute(self) };
        data
    }

    fn dispatch(&self, _event: &Events) -> Result<i32, Error> {
        log::debug!("job manager data dispatch");
        self.run();
        Ok(0)
    }
}

impl TableSubscribe<String, Rc<UnitX>> for JobManagerData {
    fn notify(&self, op: &TableOp<String, Rc<UnitX>>) {
        match op {
            TableOp::TableInsert(_, _) => {} // do nothing
            TableOp::TableRemove(_, unit) => self.remove_unit(unit),
        }
    }
}

//#[derive(Debug)]
struct JobManagerData {
    // associated objects
    db: Rc<UnitDb>,

    // owned objects
    // control
    ja: JobAlloc,

    // data
    /* job */
    jobs: JobTable,  // (relative) stable
    stage: JobTable, // temporary

    // status
    running: RefCell<bool>,
    text: RefCell<Option<(Rc<UnitX>, UnitActiveState, UnitActiveState, UnitNotifyFlags)>>, // (unit, os, ns, flags) for synchronous finish

    // statistics
    stat: JobStat,
}

// the declaration "pub(self)" is for identification only.
impl JobManagerData {
    pub(self) fn new(dbr: &Rc<UnitDb>) -> JobManagerData {
        JobManagerData {
            db: Rc::clone(dbr),

            ja: JobAlloc::new(),

            jobs: JobTable::new(),
            stage: JobTable::new(),

            running: RefCell::new(false),
            text: RefCell::new(None),

            stat: JobStat::new(),
        }
    }

    pub(self) fn exec(
        &self,
        config: &JobConf,
        mode: JobMode,
        affect: &mut JobAffect,
    ) -> Result<(), JobErrno> {
        job_trans_check_input(config, mode)?;

        self.stage.clear(); // clear stage first: make rollback simple

        // build changes in stage
        job_transaction::job_trans_expand(&self.stage, &self.ja, &self.db, config, mode)?;
        job_transaction::job_trans_affect(&self.stage, &self.ja, &self.db, config, mode)?;
        job_transaction::job_trans_verify(&self.stage, &self.jobs, mode)?;

        // commit stage to jobs
        let (add_jobs, del_jobs, update_jobs) = self.jobs.commit(&self.stage, mode)?;

        // clear stage
        self.stage.clear();

        // update statistics
        self.stat
            .update_changes(&(&add_jobs, &del_jobs, &update_jobs));
        self.stat.update_stage_wait(del_jobs.len(), false); // commit-del[wait->end]: decrease 'wait'
        self.stat.update_stage_wait(add_jobs.len(), true); // commit-add[init->wait]: increase 'wait'

        // output
        affect.record(&(add_jobs, del_jobs, update_jobs));

        Ok(())
        // if it's successful, all jobs expanded would be inserted in 'self.jobs', otherwise(failed) they would be cleared next time.
    }

    pub(self) fn notify(&self, config: &JobConf, mode: JobMode) -> Result<(), JobErrno> {
        if config.get_kind() != JobKind::JobReload {
            return Err(JobErrno::JobErrInput);
        }

        self.do_notify(config, Some(mode));
        Ok(())
    }

    pub(self) fn run(&self) {
        loop {
            // try to trigger something to run
            *self.text.borrow_mut() = None; // reset every time
            *self.running.borrow_mut() = true;
            let trigger_ret = self.jobs.try_trigger(&self.db);
            *self.running.borrow_mut() = false;

            if let Some((trigger_info, merge_trigger)) = trigger_ret {
                // something is triggered in this round
                // update statistics
                self.stat.update_change(&(&None, &merge_trigger, &None));
                self.stat
                    .update_stage_wait(trigger_info.is_some().into(), false); // trigger-someone[wait->run]: decrease 'wait'
                self.stat
                    .update_stage_run(trigger_info.is_some().into(), true); // trigger-someone[wait->run]: increase 'run'

                // try to finish it now in two case, and the case coming from unit has higher priority
                // case 1. the job has been finished synchronously in context;
                // case 2. the trigger is finished(failed or over);
                if let Some((unit, os, ns, flags)) = self.text.take() {
                    // case 1
                    self.do_try_finish(&unit, os, ns, flags);
                    *self.text.borrow_mut() = None;
                }

                if let Some((t_jinfo, Some(tfinish_r))) = trigger_info {
                    // case 2
                    self.do_remove(&t_jinfo, tfinish_r, false);
                }
            } else {
                // nothing is triggered in this round
                break;
            }
        }
    }

    pub(self) fn try_finish(
        &self,
        unit: &Rc<UnitX>,
        os: UnitActiveState,
        ns: UnitActiveState,
        flags: UnitNotifyFlags,
    ) -> Result<(), JobErrno> {
        // in order to simplify the mechanism, the running(trigger) and ending(finish) processes need to be isolated.
        if *self.running.borrow() {
            // (synchronous)finish in context
            if self.text.borrow().is_some() {
                // the unit has been finished already
                return Err(JobErrno::JobErrInput);
            }

            *self.text.borrow_mut() = Some((Rc::clone(unit), os.clone(), ns.clone(), flags));
        // update and record it.
        } else {
            // (asynchronous)finish not in context
            self.do_try_finish(unit, os, ns, flags); // do it
        }

        Ok(())
    }

    pub(self) fn remove(&self, id: u32) -> Result<(), JobErrno> {
        assert!(!*self.running.borrow());

        let job_info = self.jobs.get(id);
        if job_info.is_none() {
            return Err(JobErrno::JobErrNotExisted);
        }

        if self.jobs.is_trigger(id) {
            return Err(JobErrno::JobErrNotSupported);
        }

        if !self.jobs.is_suspend(id) {
            return Err(JobErrno::JobErrInternel);
        }

        self.do_remove(&job_info.unwrap(), JobResult::JobCancelled, true);
        Ok(())
    }

    pub(self) fn get_jobinfo(&self, id: u32) -> Option<JobInfo> {
        self.jobs.get(id)
    }

    pub(self) fn is_jobs_ready(&self) -> bool {
        self.jobs.is_ready()
    }

    pub(self) fn get_suspends(&self, unit: &Rc<UnitX>) -> Option<JobInfo> {
        self.jobs.get_suspend(unit, JobKind::JobStop)
    }

    fn remove_unit(&self, unit: &UnitX) {
        // delete related jobs
        let (del_trigger, del_suspends) = self.jobs.remove_unit(unit);

        // update statistics
        self.stat.update_change(&(&None, &del_trigger, &None));
        self.stat
            .update_changes(&(&Vec::new(), &del_suspends, &Vec::new()));
        self.stat
            .update_stage_run(del_trigger.is_none().into(), false); // remove-unit[run->end]: decrease 'run'
        self.stat.update_stage_wait(del_suspends.len(), false); // remove-unit[wait->end]: decrease 'wait'
    }

    fn do_try_finish(
        &self,
        unit: &Rc<UnitX>,
        os: UnitActiveState,
        ns: UnitActiveState,
        flags: UnitNotifyFlags,
    ) {
        let mut generated = false;
        if let Some((trigger, pause)) = self.jobs.get_trigger_info(unit) {
            generated = match pause {
                true => {
                    self.jobs.resume_unit(unit);
                    true
                }
                false => {
                    let (suggest_r, suggest_g) =
                        job_entry::job_process_unit(trigger.run_kind, ns, flags);
                    if let Some(result) = suggest_r {
                        // finish it when 'finish' is suggested
                        self.del_trigger(&trigger, result);
                    }
                    suggest_g
                }
            };
        }

        // simulate job events, which are not generated by the job.
        if !generated {
            self.simulate_job_notify(unit, os, ns);
        }

        // start on previous result
        self.unit_start_on(unit, os, ns, flags);
    }

    fn do_remove(&self, job_info: &JobInfo, result: JobResult, force: bool) {
        // delete itself
        if self.jobs.is_trigger(job_info.id) {
            self.del_trigger(job_info, result);
        } else {
            self.del_suspends(job_info, result);
        }

        // simulate and notify unit events, which are not generated by the unit.
        self.simulate_unit_notify(&job_info.unit, result, force);
    }

    fn del_trigger(&self, job_info: &JobInfo, result: JobResult) {
        // delete itself
        let del_trigger = self.jobs.finish_trigger(&self.db, &job_info.unit, result);

        // remove relational jobs on failure
        let remove_jobs = match result {
            JobResult::JobDone => Vec::new(),
            _ => job_transaction::job_trans_fallback(
                &self.jobs,
                &self.db,
                &job_info.unit,
                job_info.run_kind,
            ),
        };

        // update statistics
        self.stat.update_change(&(&None, &del_trigger, &None));
        self.stat
            .update_changes(&(&Vec::new(), &remove_jobs, &Vec::new()));
        self.stat
            .update_stage_wait(del_trigger.is_none().into(), true); // finish-retrigger(the trigger has not been deleted)[run->wait]: increase 'wait'
        self.stat.update_stage_wait(remove_jobs.len(), false); // finish-remove[wait->end]: decrease 'wait'
        self.stat.update_stage_run(1, false); // finish-someone[run->wait|end]: decrease 'run'
    }

    fn del_suspends(&self, job_info: &JobInfo, result: JobResult) {
        let mut del_jobs = Vec::new();

        // delete itself
        del_jobs.append(&mut self.jobs.remove_suspends(
            &self.db,
            &job_info.unit,
            job_info.kind,
            None,
            result,
        ));

        // remove relational jobs on failure
        if result != JobResult::JobDone {
            del_jobs.append(&mut job_transaction::job_trans_fallback(
                &self.jobs,
                &self.db,
                &job_info.unit,
                job_info.run_kind,
            ));
        }

        // update statistics
        self.stat
            .update_changes(&(&Vec::new(), &del_jobs, &Vec::new()));
        self.stat.update_stage_wait(del_jobs.len(), false); // remove-del[wait->end]: decrease 'wait'
    }

    fn simulate_job_notify(&self, unit: &Rc<UnitX>, os: UnitActiveState, ns: UnitActiveState) {
        match (os, ns) {
            (
                UnitActiveState::UnitInActive | UnitActiveState::UnitFailed,
                UnitActiveState::UnitActive | UnitActiveState::UnitActivating,
            ) => self.do_notify(&JobConf::new(Rc::clone(unit), JobKind::JobStart), None),
            (
                UnitActiveState::UnitActive | UnitActiveState::UnitActivating,
                UnitActiveState::UnitInActive | UnitActiveState::UnitDeActivating,
            ) => self.do_notify(&JobConf::new(Rc::clone(unit), JobKind::JobStop), None),
            _ => {} // do nothing
        }
    }

    fn simulate_unit_notify(&self, unit: &Rc<UnitX>, result: JobResult, force: bool) {
        // OnFailure=
        if !force {
            // is forced removement a failure?
            if result != JobResult::JobDone {
                if let JobMode::JobFail = unit
                    .get_config()
                    .config_data()
                    .borrow()
                    .Unit
                    .OnFailureJobMode
                {
                    self.exec_on(
                        Rc::clone(unit),
                        UnitRelationAtom::UnitAtomOnFailure,
                        JobMode::JobFail,
                    );
                }
            }
        }

        // trigger-notify
        for other in self
            .db
            .dep_gets_atom(unit, UnitRelationAtom::UnitAtomTriggeredBy)
        {
            other.trigger(unit);
        }
    }

    fn unit_start_on(
        &self,
        unit: &Rc<UnitX>,
        os: UnitActiveState,
        ns: UnitActiveState,
        flags: UnitNotifyFlags,
    ) {
        // OnFailure=
        if ns != os && !flags.intersects(UnitNotifyFlags::UNIT_NOTIFY_WILL_AUTO_RESTART) {
            match ns {
                UnitActiveState::UnitFailed => {
                    if let JobMode::JobFail = unit
                        .get_config()
                        .config_data()
                        .borrow()
                        .Unit
                        .OnFailureJobMode
                    {
                        self.exec_on(
                            Rc::clone(unit),
                            UnitRelationAtom::UnitAtomOnFailure,
                            JobMode::JobFail,
                        );
                    }
                }
                _ => {}
            };
        }

        // OnSuccess=
        // if ns == UnitActiveState::UnitInActive
        //     && flags & UnitNotifyFlags::UNIT_NOTIFY_WILL_AUTO_RESTART == 0
        if ns == UnitActiveState::UnitInActive
            && !flags.intersects(UnitNotifyFlags::UNIT_NOTIFY_WILL_AUTO_RESTART)
        {
            match os {
                UnitActiveState::UnitFailed
                | UnitActiveState::UnitInActive
                | UnitActiveState::UnitMaintenance => {}
                _ => {
                    if let JobMode::JobFail = unit
                        .get_config()
                        .config_data()
                        .borrow()
                        .Unit
                        .OnFailureJobMode
                    {
                        self.exec_on(
                            Rc::clone(unit),
                            UnitRelationAtom::UnitAtomOnSuccess,
                            JobMode::JobFail,
                        );
                    }
                }
            };
        }
    }

    fn exec_on(&self, unit: Rc<UnitX>, atom: UnitRelationAtom, mode: JobMode) {
        let (configs, mode) = job_notify::job_notify_result(&self.db, unit, atom, mode);
        for config in configs.iter() {
            if let Err(_e) = self.exec(config, mode, &mut JobAffect::new(false)) {
                // debug
            }
        }
    }

    fn do_notify(&self, config: &JobConf, mode_option: Option<JobMode>) {
        let targets = job_notify::job_notify_event(&self.db, config, mode_option);
        for (config, mode) in targets.iter() {
            if let Err(_e) = self.exec(config, *mode, &mut JobAffect::new(false)) {
                // debug
            }
        }
    }
}

fn jobs_2_jobinfo(jobs: &Vec<Rc<Job>>) -> Vec<JobInfo> {
    jobs.iter().map(|jr| JobInfo::map(jr)).collect::<Vec<_>>()
}

fn job_trans_check_input(config: &JobConf, mode: JobMode) -> Result<(), JobErrno> {
    let kind = config.get_kind();
    let unit = config.get_unit();

    if mode == JobMode::JobIsolate {
        if kind != JobKind::JobStart {
            return Err(JobErrno::JobErrInput);
        }

        if let false = unit.get_config().config_data().borrow().Unit.AllowIsolate {
            return Err(JobErrno::JobErrInput);
        }
    }

    if mode == JobMode::JobTrigger {
        if kind != JobKind::JobStop {
            return Err(JobErrno::JobErrInput);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::data::{DataManager, UnitRelations};
    use crate::manager::unit::job::JobStage;
    use crate::manager::unit::uload_util::UnitFile;
    use crate::manager::unit::unit_base::UnitType;
    use crate::plugin::Plugin;
    use utils::logger;

    #[test]
    fn job_exec_single() {
        let event = Rc::new(Events::new().unwrap());
        let db = Rc::new(UnitDb::new());
        let name_test1 = String::from("test1.service");
        let unit_test1 = create_unit(&name_test1);
        let name_test2 = String::from("test2.service");
        let unit_test2 = create_unit(&name_test2);
        db.units_insert(name_test1.clone(), Rc::clone(&unit_test1));
        db.units_insert(name_test2.clone(), Rc::clone(&unit_test2));
        let jm = JobManager::new(&db, &event);

        let mut affect = JobAffect::new(true);
        let ret = jm.exec(
            &JobConf::new(Rc::clone(&unit_test1), JobKind::JobStart),
            JobMode::JobReplace,
            &mut affect,
        );
        assert!(ret.is_ok());
        assert_eq!(jm.data.jobs.len(), 1);
        assert_eq!(jm.data.jobs.ready_len(), 1);

        assert_eq!(affect.adds.len(), 1);
        let job_info = affect.adds.pop().unwrap();
        assert!(Rc::ptr_eq(&job_info.unit, &unit_test1));
        assert_eq!(job_info.kind, JobKind::JobStart);
        assert_eq!(job_info.run_kind, JobKind::JobStart);
        assert_eq!(job_info.stage, JobStage::JobWait);
    }

    #[test]
    fn job_exec_multi() {
        let event = Rc::new(Events::new().unwrap());
        let db = Rc::new(UnitDb::new());
        let name_test1 = String::from("test1.service");
        let unit_test1 = create_unit(&name_test1);
        let name_test2 = String::from("test2.service");
        let unit_test2 = create_unit(&name_test2);
        let relation = UnitRelations::UnitRequires;
        db.units_insert(name_test1.clone(), Rc::clone(&unit_test1));
        db.units_insert(name_test2.clone(), Rc::clone(&unit_test2));
        db.dep_insert(
            Rc::clone(&unit_test1),
            relation,
            Rc::clone(&unit_test2),
            true,
            0,
        )
        .unwrap();
        let jm = JobManager::new(&db, &event);

        let mut affect = JobAffect::new(true);
        let ret = jm.exec(
            &JobConf::new(Rc::clone(&unit_test1), JobKind::JobStart),
            JobMode::JobReplace,
            &mut affect,
        );
        assert!(ret.is_ok());
        assert_eq!(jm.data.jobs.len(), 2);
        assert_eq!(jm.data.jobs.ready_len(), 2);
        assert_eq!(affect.adds.len(), 2);
        let job_info1 = affect.adds.pop().unwrap();
        assert!(
            Rc::ptr_eq(&job_info1.unit, &unit_test1) || Rc::ptr_eq(&job_info1.unit, &unit_test2)
        );
        assert_eq!(job_info1.kind, JobKind::JobStart);
        assert_eq!(job_info1.run_kind, JobKind::JobStart);
        assert_eq!(job_info1.stage, JobStage::JobWait);
        let job_info2 = affect.adds.pop().unwrap();
        assert!(!Rc::ptr_eq(&job_info1.unit, &job_info2.unit));
        assert!(
            Rc::ptr_eq(&job_info2.unit, &unit_test1) || Rc::ptr_eq(&job_info2.unit, &unit_test2)
        );
        assert_eq!(job_info2.kind, JobKind::JobStart);
        assert_eq!(job_info2.run_kind, JobKind::JobStart);
        assert_eq!(job_info2.stage, JobStage::JobWait);
    }

    #[test]
    fn job_run_finish_single() {
        let event = Rc::new(Events::new().unwrap());
        let db = Rc::new(UnitDb::new());
        let name_test1 = String::from("test1.service");
        let unit_test1 = create_unit(&name_test1);
        let name_test2 = String::from("test2.service");
        let unit_test2 = create_unit(&name_test2);
        db.units_insert(name_test1.clone(), Rc::clone(&unit_test1));
        db.units_insert(name_test2.clone(), Rc::clone(&unit_test2));
        let jm = JobManager::new(&db, &event);

        jm.exec(
            &JobConf::new(Rc::clone(&unit_test1), JobKind::JobNop),
            JobMode::JobReplace,
            &mut JobAffect::new(false),
        )
        .unwrap();
        assert_eq!(jm.data.jobs.len(), 1);
        assert_eq!(jm.data.jobs.ready_len(), 1);

        jm.data.run();
        assert_eq!(jm.data.jobs.len(), 0);
        assert_eq!(jm.data.jobs.ready_len(), 0);
    }

    #[test]
    fn job_run_finish_multi() {
        let event = Rc::new(Events::new().unwrap());
        let db = Rc::new(UnitDb::new());
        let name_test1 = String::from("test1.service");
        let unit_test1 = create_unit(&name_test1);
        let name_test2 = String::from("test2.service");
        let unit_test2 = create_unit(&name_test2);
        db.units_insert(name_test1.clone(), Rc::clone(&unit_test1));
        db.units_insert(name_test2.clone(), Rc::clone(&unit_test2));
        let jm = JobManager::new(&db, &event);

        jm.exec(
            &JobConf::new(Rc::clone(&unit_test1), JobKind::JobNop),
            JobMode::JobReplace,
            &mut JobAffect::new(false),
        )
        .unwrap();
        jm.exec(
            &JobConf::new(Rc::clone(&unit_test2), JobKind::JobNop),
            JobMode::JobReplace,
            &mut JobAffect::new(false),
        )
        .unwrap();
        assert_eq!(jm.data.jobs.len(), 2);
        assert_eq!(jm.data.jobs.ready_len(), 2);

        jm.data.run();
        assert_eq!(jm.data.jobs.len(), 0);
        assert_eq!(jm.data.jobs.ready_len(), 0);
    }

    fn create_unit(name: &str) -> Rc<UnitX> {
        logger::init_log_with_console("test_unit_load", 4);
        log::info!("test");
        let dm = Rc::new(DataManager::new());
        let file = Rc::new(UnitFile::new());
        let unit_type = UnitType::UnitService;
        let plugins = Plugin::get_instance();
        let subclass = plugins.create_unit_obj(unit_type).unwrap();
        Rc::new(UnitX::new(
            &dm,
            &file,
            unit_type,
            name,
            subclass.into_unitobj(),
        ))
    }
}
