use super::*;

struct ArrivalTimeFilter {
    m_hat: f32,
    e: f32,
    var_v_hat: f32,
}

impl ArrivalTimeFilter {
    fn new(intergroup_delay: i64) -> ArrivalTimeFilter {
        ArrivalTimeFilter {
            m_hat: intergroup_delay as f32,
            e: INITIAL_SYSTEM_ERROR_COVARIANCE,
            var_v_hat: 0.0,
        }
    }

    /// Updates the Kalman filter, returning the previous intergroup delay variation estimate.
    fn update(&mut self, intergroup_delay: i64, min_send_interval: i64) -> f32 {
        // This is different than in the Google CC alg. draft since the interval used here is in
        // microseconds
        let alpha = (1.0 - CHI).powf(30.0 * (min_send_interval as f32) / 1e6);

        let q = STATE_NOISE_COVARIANCE;

        // m_hat, intergroup_delay, and z are in microseconds
        let z = intergroup_delay as f32 - self.m_hat;
        let z2 = z * z;

        self.var_v_hat = f32::max(1.0, alpha * self.var_v_hat + (1.0 - alpha) * z2);
        let k = (self.e + q) / (self.var_v_hat + (self.e + q));
        self.e = (1.0 - k) * (self.e + q);

        let prev = self.m_hat;
        self.m_hat = self.m_hat + z * k;
        prev
    }
}

struct DelayThreshold {
    threshold: f32,
}

impl DelayThreshold {
    fn new() -> DelayThreshold {
        DelayThreshold {
            threshold: INITIAL_DELAY_THRESHOLD_US,
        }
    }

    fn update(&mut self, interarrival_time: i64, intergroup_delay_estimate: f32) {
        let interarrival_time = interarrival_time as f32;

        let threshold_delta = intergroup_delay_estimate.abs() - self.threshold;
        if threshold_delta <= 15000.0 {
            let k = if threshold_delta < 0.0 { K_D } else { K_U };
            self.threshold = self.threshold + interarrival_time * k * threshold_delta;
            self.threshold = self.threshold.clamp(6000.0, 600000.0);
        }
    }
}

pub enum NetworkCondition {
    Underuse,
    Normal,
    Overuse,
}

pub struct DelayDetector {
    delay_threshold: DelayThreshold,
    filter: ArrivalTimeFilter,
    overuse_start: Option<TwccTime>,
}

impl DelayDetector {
    pub fn new(intergroup_delay: i64) -> DelayDetector {
        DelayDetector {
            delay_threshold: DelayThreshold::new(),
            filter: ArrivalTimeFilter::new(intergroup_delay),
            overuse_start: None,
        }
    }

    pub fn detect_network_condition(
        &mut self,
        intergroup_delay: i64,
        min_send_interval: i64,
        interarrival_time: i64,
        arrival_time: TwccTime,
    ) -> NetworkCondition {
        let prev_m = self.filter.update(intergroup_delay, min_send_interval);
        let m = self.filter.m_hat;

        self.delay_threshold
            .update(interarrival_time, m);
        let del_var_th = self.delay_threshold.threshold;

        if m > del_var_th {
            if m < prev_m {
                self.overuse_start = None;
                NetworkCondition::Normal
            } else {
                if let Some(overuse_start) = self.overuse_start {
                    let elapsed = arrival_time.small_delta_sub(overuse_start);
                    if elapsed >= OVERUSE_TIME_THRESHOLD_US {
                        return NetworkCondition::Overuse;
                    }
                } else {
                    self.overuse_start = Some(arrival_time);
                }
                NetworkCondition::Normal
            }
        } else if m < -del_var_th {
            self.overuse_start = None;
            NetworkCondition::Underuse
        } else {
            self.overuse_start = None;
            NetworkCondition::Normal
        }
    }
}
