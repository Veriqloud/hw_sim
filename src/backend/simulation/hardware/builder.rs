use crate::backend::simulation::Hardware;

/// Mapped on [Hardware].
#[derive(Debug)]
pub struct HardwareBuilder {
    // HW
    pulse_distance: f64,
    // HW
    gc_offset: u64,
}

impl Default for HardwareBuilder {
    fn default() -> Self {
        HardwareBuilder {
            pulse_distance: 1e-8,
            gc_offset: 0,
        }
    }
}

impl HardwareBuilder {
    pub fn new() -> HardwareBuilder {
        Default::default()
    }

    pub fn with_pulse_distance(&mut self, pulse_distance: f64) -> &mut Self {
        self.pulse_distance = pulse_distance;
        self
    }

    pub fn with_gc_offset(&mut self, gc_offset: u64) -> &mut Self {
        self.gc_offset = gc_offset;
        self
    }

    pub fn build(&self) -> Hardware {
        Hardware {
            pulse_distance: self.pulse_distance,
            gc_offset: self.gc_offset,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use crate::backend::simulation::hardware::{builder::HardwareBuilder, Hardware};

    #[test]
    fn hardware_builder() {
        let hb = HardwareBuilder::new()
            .with_pulse_distance(10.0)
            .with_gc_offset(32)
            .build();

        assert_eq!(
            Hardware {
                pulse_distance: 10.0,
                gc_offset: 32,
            },
            hb
        )
    }
}
