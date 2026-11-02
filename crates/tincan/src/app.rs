//! Application-level Qt calls not wrapped by cxx-qt-lib.

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("helpers.h");

        /// Create the application, load `qml_url` and run the event loop.
        fn run_application(qml_url: &str) -> i32;
        fn quit_application();
    }
}
