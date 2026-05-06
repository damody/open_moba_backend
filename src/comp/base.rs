# ![功能（基本）]


#[cfg(feature = "tracy")] pub use tracy_client;

/// 允許下游 crates 根據 tracy 是否有條件地執行操作
/// 無需公開貨物功能即可啟用。
pub const TRACY_ENABLED: bool = cfg!(feature = "tracy");

#[cfg(not(feature = "tracy"))]
macro_rules! plot {
    ($name:expr, $value:expr) => {
        // 類型檢查
        let _: f64 = $value;
    };
}
#[cfg(feature = "tracy")]
macro_rules! plot {
    ($name:expr, $value:expr) => {{
        use $crate::tracy_client::{create_plot, Plot};
        static PLOT: Plot = create_plot!($name);
        PLOT.point($value);
    }};
}

// 調試或測試時出現恐慌，發佈時發出警告

macro_rules! dev_panic {
    ($msg:expr) => {
        if cfg!(any(debug_assertions, test)) {
            panic!("{}", $msg);
        } else {
            tracing::error!("{}", $msg);
        }
    };

    ($msg:expr, or return $result:expr) => {
        if cfg!(any(debug_assertions, test)) {
            panic!("{}", $msg);
        } else {
            tracing::warn!("{}", $msg);
            return $result;
        }
    };
}

// https://discordapp.com/channels/676678179678715904/676685797524766720/723358438943621151
#[cfg(not(feature = "tracy"))]
macro_rules! span {
    ($guard_name:tt, $level:ident, $name:expr, $($fields:tt)*) => {
        let span = tracing::span!(tracing::Level::$level, $name, $($fields)*);
        let $guard_name = span.enter();
    };
    ($guard_name:tt, $level:ident, $name:expr) => {
        let span = tracing::span!(tracing::Level::$level, $name);
        let $guard_name = span.enter();
    };
    ($guard_name:tt, $name:expr) => {
        let span = tracing::span!(tracing::Level::TRACE, $name);
        let $guard_name = span.enter();
    };
    ($guard_name:tt, $no_tracy_name:expr, $tracy_name:expr) => {
        $crate::span!($guard_name, $no_tracy_name);
    };
}
pub(crate) use span;
#[cfg(feature = "tracy")]

macro_rules! span {
    ($guard_name:tt, $level:ident, $name:expr, $($fields:tt)*) => {
        let span = tracing::span!(tracing::Level::$level, $name, $($fields)*);
        let $guard_name = span.enter();
    };
    ($guard_name:tt, $level:ident, $name:expr) => {
        let span = tracing::span!(tracing::Level::$level, $name);
        let $guard_name = span.enter();
    };
    ($guard_name:tt, $name:expr) => {
        // 直接使用“tracy_client”來減少開銷以獲得更好的時序
        let $guard_name = $crate::tracy_client::Span::new(
            $name,
            "",
            module_path!(),
            line!(),
            // 沒有呼叫堆疊，因為這會產生很大的開銷
            0,
        );
    };
    ($guard_name:tt, $no_tracy_name:expr, $tracy_name:expr) => {
        $crate::span!($guard_name, $tracy_name);
    };
}

#[cfg(feature = "tracy")]
pub struct ProfSpan(pub tracy_client::Span);
#[cfg(not(feature = "tracy"))]
pub struct ProfSpan;

/// 與 span 巨集類似，但僅在分析時使用，而不是在常規追蹤中使用
/// 營運

#[cfg(not(feature = "tracy"))]
macro_rules! prof_span {
    ($guard_name:tt, $name:expr) => {
        let $guard_name = $crate::ProfSpan;
    };
    // 當您希望將防護裝置放在範圍末端時的簡寫
    // 手動控制它
    ($name:expr) => {};
}
pub(crate) use prof_span;
/// 與 span 巨集類似，但僅在分析時使用，而不是在常規追蹤中使用
/// 營運

#[cfg(feature = "tracy")]
macro_rules! prof_span {
    ($guard_name:tt, $name:expr) => {
        let $guard_name = $crate::ProfSpan($crate::tracy_client::Span::new(
            $name,
            "",
            module_path!(),
            line!(),
            // 沒有呼叫堆疊，因為這會產生很大的開銷
            0,
        ));
    };
    // 當您希望將防護裝置放在範圍末端時的簡寫
    // 手動控制它
    ($name:expr) => {
        $crate::prof_span!(_guard, $name);
    };
}

/// 沒有警衛，但這確實是警衛
pub struct GuardlessSpan {
    span: tracing::Span,
    subscriber: tracing::Dispatch,
}

impl GuardlessSpan {
    pub fn new(span: tracing::Span) -> Self {
        let subscriber = tracing::dispatcher::get_default(|d| d.clone());
        if let Some(id) = span.id() {
            subscriber.enter(&id)
        }
        Self { span, subscriber }
    }
}

impl Drop for GuardlessSpan {
    fn drop(&mut self) {
        if let Some(id) = self.span.id() {
            self.subscriber.exit(&id)
        }
    }
}


macro_rules! no_guard_span {
    ($level:ident, $name:expr, $($fields:tt)*) => {
        GuardlessSpan::new(
            tracing::span!(tracing::Level::$level, $name, $($fields)*)
        )
    };
    ($level:ident, $name:expr) => {
        GuardlessSpan::new(
            tracing::span!(tracing::Level::$level, $name)
        )
    };
    ($name:expr) => {
        GuardlessSpan::new(
            tracing::span!(tracing::Level::TRACE, $name)
        )
    };
}

