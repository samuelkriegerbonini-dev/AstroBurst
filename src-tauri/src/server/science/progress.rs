use std::sync::Arc;

use astroburst_lib::infra::progress::{ProgressHandle, ProgressPayload, ProgressSink};

use crate::job::Job;

pub(crate) fn job_progress(job: &Arc<Job>, total: u64) -> ProgressHandle {
    let sink: ProgressSink = {
        let job = Arc::clone(job);
        Arc::new(move |payload: &ProgressPayload| job.progress(payload.percent, &payload.stage))
    };
    let job = Arc::clone(job);
    ProgressHandle::with_sink(sink, total)
        .with_cancel_source(Arc::new(move || job.cancel.is_cancelled()))
}
