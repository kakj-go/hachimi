//! Persistent prompt-task scheduling built on the shared Hachimi Agent runtime.

mod calendar;
mod event;
mod service;
mod service_helpers;

pub use calendar::{
    BundledIanaTimeZoneResolver, CalendarError, TimeZoneResolver, error_code, occurrences_after,
    preview_schedule,
};
pub use service::{
    Clock, NoopNotificationAdapter, NotificationAdapter, NotificationFuture, ScheduleLaunchError,
    ScheduleLaunchFuture, ScheduleRunCompletion, ScheduleRunLauncher, SchedulerError,
    SchedulerHandle, SchedulerService, SystemClock, TaskNotification,
    normalize_schedule_definition,
};
