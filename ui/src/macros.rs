#[macro_export]
macro_rules! block_on {
    ($future:expr) => {
        glib::MainContext::ref_thread_default().block_on($future);
    };
}

#[macro_export]
macro_rules! lock_async_mutex {
    ($mutex:expr) => {
        $crate::block_on!($mutex.lock());
    };
}

#[macro_export]
macro_rules! spawn {
    ($future:expr) => {
        glib::MainContext::ref_thread_default().spawn_local($future);
    };
}
