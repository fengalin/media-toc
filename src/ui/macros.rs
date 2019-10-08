/// This macro allows declaring a closure which will borrow the specified `main_ctrl_rc`
/// The following variantes are available:
///
/// - Borrow as immutable
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => |&main_ctrl| main_ctrl.about()
/// )
/// ```
///
/// - Borrow as mutable
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => |&mut main_ctrl| main_ctrl.quit()
/// )
/// ```
///
/// - Borrow as mutable with argument(s) (also available as immutable)
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => |&mut main_ctrl, event| main_ctrl.handle_media_event(event)
/// )
/// ```
///
/// - Borrow as mutable and trigger asynchronously (also available as immutable and with argument(s))
/// ```
/// with_main_ctrl!(
///     main_ctrl_rc => async |&mut main_ctrl| main_ctrl.about()
/// )
/// ```
#[macro_export]
macro_rules! with_main_ctrl {
    (@arg _) => ( _ );
    (@arg $x:ident) => ( $x );

    ($main_ctrl_rc:ident => move |&$main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                let $main_ctrl = main_ctrl_rc.borrow();
                $body
            }
        }
    };
    ($main_ctrl_rc:ident => move |&$main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move |$($crate::with_main_ctrl!(@arg $p),)+| {
                let $main_ctrl = main_ctrl_rc.borrow();
                $body
            }
        }
    };
    ($main_ctrl_rc:ident => move |&mut $main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                $body
            }
        }
    };
    ($main_ctrl_rc:ident => move |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move |$($crate::with_main_ctrl!(@arg $p),)+| {
                let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                $body
            }
        }
    };
    (
        $main_ctrl_rc:ident => try move |&$main_ctrl:ident| $body:block
        else $else:block
    ) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                if let Ok($main_ctrl) = main_ctrl_rc.try_borrow() {
                    $body
                } else $else
            }
        }
    };
    ($main_ctrl_rc:ident => try move |&$main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                if let Ok($main_ctrl) = main_ctrl_rc.try_borrow() {
                    $body
                }
            }
        }
    };
    ($main_ctrl_rc:ident => try move |&$main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move |$($crate::with_main_ctrl!(@arg $p),)+| {
                if let Ok($main_ctrl) = main_ctrl_rc.try_borrow() {
                    $body
                }
            }
        }
    };
    (
        $main_ctrl_rc:ident => try move |&mut $main_ctrl:ident| $body:block
        else $else:block
    ) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                if let Ok(mut $main_ctrl) = main_ctrl_rc.try_borrow_mut() {
                    $body
                } else $else
            }
        }
    };
    ($main_ctrl_rc:ident => try move |&mut $main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                if let Ok(mut $main_ctrl) = main_ctrl_rc.try_borrow_mut() {
                    $body
                }
            }
        }
    };
    ($main_ctrl_rc:ident => try move |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move |$($crate::with_main_ctrl!(@arg $p),)+| {
                if let Ok(mut $main_ctrl) = main_ctrl_rc.try_borrow_mut() {
                    $body
                }
            }
        }
    };
    ($main_ctrl_rc:ident => move async |&mut $main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                let main_ctrl_rc = std::rc::Rc::clone(&main_ctrl_rc);
                async move {
                    let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                    $body
                }
            }
        }
    };
    ($main_ctrl_rc:ident => move async |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move |$($crate::with_main_ctrl!(@arg $p),)+| {
                let main_ctrl_rc = std::rc::Rc::clone(&main_ctrl_rc);
                async move {
                    let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                    $body
                }
            }
        }
    };
    ($main_ctrl_rc:ident => move async boxed_local |&mut $main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move || {
                let main_ctrl_rc = std::rc::Rc::clone(&main_ctrl_rc);
                async move {
                    let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                    $body
                }.boxed_local()
            }
        }
    };
    ($main_ctrl_rc:ident => move async boxed_local |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            move |$($crate::with_main_ctrl!(@arg $p),)+| {
                let main_ctrl_rc = std::rc::Rc::clone(&main_ctrl_rc);
                async move {
                    let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                    $body
                }.boxed_local()
            }
        }
    };
}

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

#[macro_export]
macro_rules! spawn_with_main_ctrl {
    (@arg _) => ( _ );
    (@arg $x:ident) => ( $x );

    ($main_ctrl_rc:ident => move async |&$main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            $crate::spawn!(async move {
                let $main_ctrl = main_ctrl_rc.borrow();
                $body
            })
        }
    };
    ($main_ctrl_rc:ident => move async |&$main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            $crate::spawn!(async move {
                let $main_ctrl = main_ctrl_rc.borrow();
                $body
            })
        }
    };
    ($main_ctrl_rc:ident => move async |&mut $main_ctrl:ident| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            $crate::spawn!(async move {
                let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                $body
            })
        }
    };
    ($main_ctrl_rc:ident => move async |&mut $main_ctrl:ident, $($p:tt),+| $body:expr) => {
        {
            let main_ctrl_rc = std::rc::Rc::clone(&$main_ctrl_rc);
            $crate::spawn!(async move {
                let mut $main_ctrl = main_ctrl_rc.borrow_mut();
                $body
            })
        }
    };
}

#[macro_export]
macro_rules! call_async_with {
    (@arg _) => ( _ );
    (@arg $arg:ident) => ( $arg );

    ( ($( $clone:ident ),+ ) => move || $body:expr) => {
        {
            $( let $clone = $clone.clone(); )+
            move || {
                $( let $clone = $clone.clone(); )+
                $body
            }
        }
    };
    ( ($( $clone:ident ),+ ) => move |($($arg:tt),+)| $body:expr) => {
        {
            $( let $clone = $clone.clone(); )+
            move |($(call_async_with!(@arg $arg),)+)| {
                $( let $clone = $clone.clone(); )+
                $body
            }
        }
    };
    ( ($( $clone:ident ),+ ) => move |$($arg:tt),+| $body:expr) => {
        {
            $( let $clone = $clone.clone(); )+
            move |$(call_async_with!(@arg $arg),)+| {
                $( let $clone = $clone.clone(); )+
                $body
            }
        }
    };
    ( ($( $clone:ident ),+ ) => move async || $body:expr) => {
        {
            $( let $clone = $clone.clone(); )+
            move || {
                $( let $clone = $clone.clone(); )+
                async move {
                    $body
                }
            }
        }
    };
    ( ($( $clone:ident ),+ ) => move async |$($arg:tt),*| $body:expr) => {
        {
            $( let $clone = $clone.clone(); )+
            move |$(call_async_with!(@arg $arg),)*| {
                $( let $clone = $clone.clone(); )+
                async move {
                    $body
                }
            }
        }
    };
    ( ($( $clone:ident ),+ ) => move async boxed_local || $body:expr) => {
        {
            $( let $clone = $clone.clone(); )+
            move || {
                $( let $clone = $clone.clone(); )+
                async move {
                    $body
                }.boxed_local()
            }
        }
    };
    ( ($( $clone:ident ),+ ) => move async boxed_local |$($arg:tt),*| $body:expr) => {
        {
            $( let $clone = $clone.clone(); )+
            move |$(call_async_with!(@arg $arg),)*| {
                $( let $clone = $clone.clone(); )+
                async move {
                    $body
                }.boxed_local()
            }
        }
    };
}
