pub mod builder;
pub mod errors;
pub mod modulator_state;

#[derive(PartialEq, Debug, Clone, Default)]
pub struct Hardware {
    /// Qubit distance in seconds
    pub pulse_distance: f64,

    /// Distance to Bob in meters.
    /// physical_distance: f64,
    /// Global counter offset
    pub gc_offset: u64,
}
