mod app;
mod config;
mod jj;
mod model;
mod ui;

fn main() -> anyhow::Result<()> {
    app::run()
}
