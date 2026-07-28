//! Persistent prompt-task scheduling built on the shared Hachimi Agent runtime.

mod calendar;
mod service;

pub use calendar::{
    BundledIanaTimeZoneResolver, CalendarError, TimeZoneResolver, error_code, occurrences_after,
    preview_schedule,
};
pub use service::{
    Clock, NoopNotificationAdapter, NotificationAdapter, NotificationFuture, ScheduleLaunchError,
    ScheduleLaunchFuture, ScheduleRunCompletion, ScheduleRunLauncher, SchedulerError,
    SchedulerHandle, SchedulerService, SystemClock, TaskNotification,
};
