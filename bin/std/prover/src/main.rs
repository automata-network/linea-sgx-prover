use app_prover::App;
use std::sync::Arc;

fn main() {
    glog::init();
    let app = Arc::new(App::default());
    app::set_ctrlc({
        let app = app.clone();
        move || {
            app::App::terminate(app.as_ref());
        }
    });
    app::run_std(app.as_ref());
}
