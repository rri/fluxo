//! fluxo's forecasting engine: a Monte Carlo simulation over goals, tasks and risks.
//!
//! Pure computation with no I/O. Build a [`Model`], [`compile`](Model::compile) it, then
//! [`run`](Compiled::run) it to get P50/P85 finishes and criticality for every top-level goal.

pub mod engine;
pub mod model;
pub mod synth;

pub use engine::{Compiled, Forecast, GoalForecast, Settings};
pub use model::{Clarity, Coverage, Kind, Model, ModelError, Node, NodeId, Risk, Task};
