#[cfg(feature = "light-2d")]
pub mod box2d_backend {
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Body2D {
        pub position: [f32; 2],
        pub velocity: [f32; 2],
    }

    #[derive(Debug, Default)]
    pub struct World2D {
        pub bodies: Vec<Body2D>,
    }

    impl World2D {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn add_body(&mut self, position: [f32; 2], velocity: [f32; 2]) -> u32 {
            let index = self.bodies.len() as u32;
            self.bodies.push(Body2D { position, velocity });
            index
        }

        pub fn step(&mut self, dt: f32) {
            for body in &mut self.bodies {
                body.position[0] += body.velocity[0] * dt;
                body.position[1] += body.velocity[1] * dt;
            }
        }
    }
}

#[cfg(not(feature = "light-2d"))]
pub mod box2d_backend {
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Stub2D;

    impl Stub2D {
        pub fn explain() -> &'static str {
            "light-2d feature is disabled; 2D physics path is compiled out"
        }
    }
}
