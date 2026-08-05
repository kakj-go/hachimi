//! Persistent prompt-task scheduling built on the shared Hachimi Agent runtime.

mod calendar;
mod event;
mod runtime;
mod service;
mod service_helpers;

pub use calendar::{
    BundledIanaTimeZoneResolver, CalendarError, TimeZoneResolver, error_code, occurrences_after,
    preview_schedule,
};
pub use runtime::{SchedulerHandle, SchedulerRuntimeEvent, SchedulerRuntimeObserver};
pub use service::{
    Clock, NoopNotificationAdapter, NotificationAdapter, NotificationFuture, ScheduleLaunchError,
    ScheduleLaunchFuture, ScheduleRunCompletion, ScheduleRunLauncher, SchedulerError,
    SchedulerService, SystemClock, TaskNotification, normalize_schedule_definition,
};
