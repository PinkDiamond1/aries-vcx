#[cfg(target_os = "android")]
extern crate android_logger;
extern crate env_logger;
extern crate libc;
extern crate log;

use chrono::format::{DelayedFormat, StrftimeItems};
use std::env;
use std::ffi::CString;
use std::io::Write;
use std::ptr;

pub use aries_vcx::indy_sys::{
    logger::{EnabledCB, FlushCB, LogCB},
    CVoid,
};
use aries_vcx::libindy;

use crate::api_lib::utils::cstring::CStringUtils;
use aries_vcx::error::{VcxError, VcxErrorKind, VcxResult};

#[allow(unused_imports)]
#[cfg(target_os = "android")]
use self::android_logger::Filter;
use self::env_logger::fmt::Formatter;
use self::env_logger::Builder as EnvLoggerBuilder;
use self::libc::c_char;
use self::log::{Level, LevelFilter, Metadata, Record};
use crate::chrono::Local;

pub static mut LOGGER_STATE: LoggerState = LoggerState::Default;
static mut CONTEXT: *const CVoid = ptr::null();
static mut ENABLED_CB: Option<EnabledCB> = None;
static mut LOG_CB: Option<LogCB> = None;
static mut FLUSH_CB: Option<FlushCB> = None;

#[derive(Debug, PartialEq)]
pub enum LoggerState {
    Default,
    Custom,
}

impl LoggerState {
    pub fn get(&self) -> (*const CVoid, Option<EnabledCB>, Option<LogCB>, Option<FlushCB>) {
        match self {
            LoggerState::Default => (
                ptr::null(),
                Some(LibvcxDefaultLogger::enabled),
                Some(LibvcxDefaultLogger::log),
                Some(LibvcxDefaultLogger::flush),
            ),
            LoggerState::Custom => unsafe { (CONTEXT, ENABLED_CB, LOG_CB, FLUSH_CB) },
        }
    }
}

pub struct LibvcxLogger {
    context: *const CVoid,
    enabled: Option<EnabledCB>,
    log: LogCB,
    flush: Option<FlushCB>,
}

impl LibvcxLogger {
    fn new(context: *const CVoid, enabled: Option<EnabledCB>, log: LogCB, flush: Option<FlushCB>) -> Self {
        LibvcxLogger {
            context,
            enabled,
            log,
            flush,
        }
    }

    pub fn init(
        context: *const CVoid,
        enabled: Option<EnabledCB>,
        log: LogCB,
        flush: Option<FlushCB>,
    ) -> VcxResult<()> {
        trace!("LibvcxLogger::init >>>");
        let logger = LibvcxLogger::new(context, enabled, log, flush);
        log::set_boxed_logger(Box::new(logger)).map_err(|err| {
            VcxError::from_msg(
                VcxErrorKind::LoggingError,
                format!("Setting logger failed with: {}", err),
            )
        })?;
        log::set_max_level(LevelFilter::Trace);
        libindy::utils::logger::set_logger(log::logger())
            .map_err(|err| err.map(aries_vcx::error::VcxErrorKind::LoggingError, "Setting logger failed"))?;

        unsafe {
            LOGGER_STATE = LoggerState::Custom;
            CONTEXT = context;
            ENABLED_CB = enabled;
            LOG_CB = Some(log);
            FLUSH_CB = flush
        }

        Ok(())
    }
}

unsafe impl Sync for LibvcxLogger {}

unsafe impl Send for LibvcxLogger {}

impl log::Log for LibvcxLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        if let Some(enabled_cb) = self.enabled {
            let level = metadata.level() as u32;
            let target = CString::new(metadata.target()).unwrap();

            enabled_cb(self.context, level, target.as_ptr())
        } else {
            true
        }
    }

    fn log(&self, record: &Record) {
        let log_cb = self.log;

        let level = record.level() as u32;
        let target = CString::new(record.target()).unwrap();
        let message = CString::new(record.args().to_string()).unwrap();

        let module_path = record.module_path().map(|a| CString::new(a).unwrap());
        let file = record.file().map(|a| CString::new(a).unwrap());
        let line = record.line().unwrap_or(0);

        log_cb(
            self.context,
            level,
            target.as_ptr(),
            message.as_ptr(),
            module_path.as_ref().map(|p| p.as_ptr()).unwrap_or(ptr::null()),
            file.as_ref().map(|p| p.as_ptr()).unwrap_or(ptr::null()),
            line,
        )
    }

    fn flush(&self) {
        if let Some(flush_cb) = self.flush {
            flush_cb(self.context)
        }
    }
}

// From: https://www.tutorialspoint.com/log4j/log4j_logging_levels.htm
//
//DEBUG	Designates fine-grained informational events that are most useful to debug an application.
//ERROR	Designates error events that might still allow the application to continue running.
//FATAL	Designates very severe error events that will presumably lead the application to abort.
//INFO	Designates informational messages that highlight the progress of the application at coarse-grained level.
//OFF	The highest possible rank and is intended to turn off logging.
//TRACE	Designates finer-grained informational events than the DEBUG.
//WARN	Designates potentially harmful situations.
pub struct LibvcxDefaultLogger;

fn _get_timestamp<'a>() -> DelayedFormat<StrftimeItems<'a>> {
    Local::now().format("%Y-%m-%d %H:%M:%S.%f")
}

fn text_format(buf: &mut Formatter, record: &Record) -> std::io::Result<()> {
    let level = buf.default_styled_level(record.level());
    writeln!(
        buf,
        "{} | {:>5} | {:<30} | {:>35}:{:<4} | {}",
        _get_timestamp(),
        level,
        record.target(),
        record.file().get_or_insert(""),
        record.line().get_or_insert(0),
        record.args()
    )
}

fn text_no_color_format(buf: &mut Formatter, record: &Record) -> std::io::Result<()> {
    let level = record.level();
    writeln!(
        buf,
        "{} | {:>5} | {:<30} | {:>35}:{:<4} | {}",
        _get_timestamp(),
        level,
        record.target(),
        record.file().get_or_insert(""),
        record.line().get_or_insert(0),
        record.args()
    )
}

impl LibvcxDefaultLogger {
    pub fn init(pattern: Option<String>) -> VcxResult<()> {
        info!("LibvcxDefaultLogger::init >>> pattern: {:?}", pattern);

        let pattern = pattern.or(env::var("RUST_LOG").ok());
        if cfg!(target_os = "android") {
            #[cfg(target_os = "android")]
            let log_filter = match pattern.as_ref() {
                Some(val) => match val.to_lowercase().as_ref() {
                    "error" => Filter::default().with_min_level(log::Level::Error),
                    "warn" => Filter::default().with_min_level(log::Level::Warn),
                    "info" => Filter::default().with_min_level(log::Level::Info),
                    "debug" => Filter::default().with_min_level(log::Level::Debug),
                    "trace" => Filter::default().with_min_level(log::Level::Trace),
                    _ => Filter::default().with_min_level(log::Level::Error),
                },
                None => Filter::default().with_min_level(log::Level::Error),
            };

            //Set logging to off when deploying production android app.
            #[cfg(target_os = "android")]
            android_logger::init_once(log_filter);
            info!("Logging for Android");
        } else {
            let formatter = match env::var("RUST_LOG_FORMATTER") {
                Ok(val) => match val.as_str() {
                    "text_no_color" => text_no_color_format,
                    _ => text_format,
                },
                _ => text_format,
            };
            EnvLoggerBuilder::new()
                .format(formatter)
                .filter(None, LevelFilter::Off)
                .parse_filters(pattern.as_ref().map(String::as_str).unwrap_or("warn"))
                .try_init()
                .map_err(|err| {
                    VcxError::from_msg(VcxErrorKind::LoggingError, format!("Cannot init logger: {:?}", err))
                })?;
        }
        libindy::utils::logger::set_default_logger(pattern.as_ref().map(String::as_str)).map_err(|err| {
            VcxError::from_msg(
                VcxErrorKind::LoggingError,
                format!("Setting default logger failed: {:?}", err),
            )
        })
    }

    extern "C" fn enabled(_context: *const CVoid, level: u32, target: *const c_char) -> bool {
        let level = get_level(level);
        let target = CStringUtils::c_str_to_str(target).unwrap().unwrap();

        let metadata: Metadata = Metadata::builder().level(level).target(target).build();

        log::logger().enabled(&metadata)
    }

    extern "C" fn log(
        _context: *const CVoid,
        level: u32,
        target: *const c_char,
        args: *const c_char,
        module_path: *const c_char,
        file: *const c_char,
        line: u32,
    ) {
        let target = CStringUtils::c_str_to_str(target).unwrap().unwrap();
        let args = CStringUtils::c_str_to_str(args).unwrap().unwrap();
        let module_path = CStringUtils::c_str_to_str(module_path).unwrap();
        let file = CStringUtils::c_str_to_str(file).unwrap();

        let level = get_level(level);

        log::logger().log(
            &Record::builder()
                .args(format_args!("{}", args))
                .level(level)
                .target(target)
                .module_path(module_path)
                .file(file)
                .line(Some(line))
                .build(),
        );
    }

    extern "C" fn flush(_context: *const CVoid) {
        log::logger().flush()
    }
}

fn get_level(level: u32) -> Level {
    match level {
        1 => Level::Error,
        2 => Level::Warn,
        3 => Level::Info,
        4 => Level::Debug,
        5 => Level::Trace,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_custom_context() -> *const CVoid {
        ptr::null()
    }

    static mut COUNT: u32 = 0;

    extern "C" fn custom_enabled(_context: *const CVoid, _level: u32, _target: *const c_char) -> bool {
        true
    }

    extern "C" fn custom_flush(_context: *const CVoid) {}

    extern "C" fn custom_log(
        _context: *const CVoid,
        _level: u32,
        _target: *const c_char,
        _message: *const c_char,
        _module_path: *const c_char,
        _file: *const c_char,
        _line: u32,
    ) {
        unsafe { COUNT = COUNT + 1 }
    }

    // #[ignore]
    // #[test]
    // #[cfg(feature = "general_test")]
    // fn test_logging_get_logger() {
    //     LibvcxDefaultLogger::init(Some("debug".to_string())).unwrap();
    //     unsafe {
    //         let (context, enabled_cb, _log_cb, _flush_cb) = LOGGER_STATE.get();
    //         assert_eq!(context, ptr::null());
    //         let target = CStringUtils::string_to_cstring("target".to_string());
    //         let level = 1;
    //         let b = LibvcxDefaultLogger::enabled(ptr::null(), 1, target.as_ptr());

    //         assert_eq!(enabled_cb.unwrap()(ptr::null(), level, target.as_ptr()), b);
    //     }
    // }

    // Can only have one test that initializes logging.
    #[ignore]
    #[test]
    #[cfg(feature = "general_test")]
    fn test_custom_logger() {
        LibvcxLogger::init(
            get_custom_context(),
            Some(custom_enabled),
            custom_log,
            Some(custom_flush),
        )
        .unwrap();
        error!("error level message"); // first call of log function
        unsafe {
            assert_eq!(COUNT, 2) // second-time log function was called inside libindy
        }
    }
}
