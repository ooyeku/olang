use std::env;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Trace => write!(f, "TRACE"),
        }
    }
}

pub struct Logger {
    level: LogLevel,
    show_timestamp: bool,
    show_module: bool,
}

impl Default for Logger {
    fn default() -> Self {
        let level = env::var("OLANG_LOG_LEVEL")
            .unwrap_or_else(|_| "WARN".to_string())
            .parse()
            .unwrap_or(LogLevel::Warn);

        let show_timestamp = env::var("OLANG_LOG_TIMESTAMP")
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        let show_module = env::var("OLANG_LOG_MODULE")
            .map(|v| v.to_lowercase() == "true")
            .unwrap_or(false);

        Self {
            level,
            show_timestamp,
            show_module,
        }
    }
}

impl std::str::FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "ERROR" => Ok(LogLevel::Error),
            "WARN" => Ok(LogLevel::Warn),
            "INFO" => Ok(LogLevel::Info),
            "DEBUG" => Ok(LogLevel::Debug),
            "TRACE" => Ok(LogLevel::Trace),
            _ => Err(format!("Invalid log level: {}", s)),
        }
    }
}

impl Logger {
    pub fn new(level: LogLevel) -> Self {
        Self {
            level,
            show_timestamp: false,
            show_module: false,
        }
    }

    pub fn with_timestamp(mut self, show: bool) -> Self {
        self.show_timestamp = show;
        self
    }

    pub fn with_module(mut self, show: bool) -> Self {
        self.show_module = show;
        self
    }

    pub fn log(&self, level: LogLevel, module: &str, message: &str) {
        if level <= self.level {
            let timestamp = if self.show_timestamp {
                format!("[{}] ", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f"))
            } else {
                String::new()
            };

            let module_info = if self.show_module {
                format!("[{}] ", module)
            } else {
                String::new()
            };

            eprintln!("{}{}{}: {}", timestamp, module_info, level, message);
        }
    }

    pub fn error(&self, module: &str, message: &str) {
        self.log(LogLevel::Error, module, message);
    }

    pub fn warn(&self, module: &str, message: &str) {
        self.log(LogLevel::Warn, module, message);
    }

    pub fn info(&self, module: &str, message: &str) {
        self.log(LogLevel::Info, module, message);
    }

    pub fn debug(&self, module: &str, message: &str) {
        self.log(LogLevel::Debug, module, message);
    }

    pub fn trace(&self, module: &str, message: &str) {
        self.log(LogLevel::Trace, module, message);
    }
}

// Global logger instance
use std::sync::OnceLock;
static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init_logger() -> &'static Logger {
    LOGGER.get_or_init(Logger::default)
}

pub fn get_logger() -> &'static Logger {
    LOGGER.get_or_init(|| Logger::new(LogLevel::Warn))
}

// Convenience macros
#[macro_export]
macro_rules! log_error {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::get_logger().error($module, &format!($($arg)*));
    };
}

#[macro_export]
macro_rules! log_warn {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::get_logger().warn($module, &format!($($arg)*));
    };
}

#[macro_export]
macro_rules! log_info {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::get_logger().info($module, &format!($($arg)*));
    };
}

#[macro_export]
macro_rules! log_debug {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::get_logger().debug($module, &format!($($arg)*));
    };
}

#[macro_export]
macro_rules! log_trace {
    ($module:expr, $($arg:tt)*) => {
        $crate::log::get_logger().trace($module, &format!($($arg)*));
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
    }

    #[test]
    fn test_log_level_parsing() {
        assert_eq!("ERROR".parse::<LogLevel>().unwrap(), LogLevel::Error);
        assert_eq!("WARN".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        assert_eq!("INFO".parse::<LogLevel>().unwrap(), LogLevel::Info);
        assert_eq!("DEBUG".parse::<LogLevel>().unwrap(), LogLevel::Debug);
        assert_eq!("TRACE".parse::<LogLevel>().unwrap(), LogLevel::Trace);
        assert!("INVALID".parse::<LogLevel>().is_err());
    }

    #[test]
    fn test_logger_creation() {
        let logger = Logger::new(LogLevel::Debug);
        assert_eq!(logger.level, LogLevel::Debug);
        assert!(!logger.show_timestamp);
        assert!(!logger.show_module);
    }

    #[test]
    fn test_logger_with_options() {
        let logger = Logger::new(LogLevel::Info)
            .with_timestamp(true)
            .with_module(true);
        assert_eq!(logger.level, LogLevel::Info);
        assert!(logger.show_timestamp);
        assert!(logger.show_module);
    }
}
