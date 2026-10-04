//! Polling cadence only. Capture and OCR discovery keep their own clocks.

const MIN_SCAN_INTERVAL_MS: u64 = 400;
const MAX_SCAN_INTERVAL_MS: u64 = 5000;
const IDLE_MIN_INTERVAL_MS: u64 = 1500;

#[derive(Debug, Default)]
pub(crate) struct ScanSchedule {
    active: bool,
}

impl ScanSchedule {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Only three recognized offers activate the faster cadence. A miss
    /// immediately returns polling to idle; this does not clear visible badges.
    pub(crate) fn observe(&mut self, has_three_offers: bool) {
        self.active = has_three_offers;
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    /// Respect the configured interval during a choice. Outside a choice,
    /// reduce polling without overriding a slower user preference.
    pub(crate) fn interval_ms(&self, base_ms: u64) -> u64 {
        let configured = base_ms.clamp(MIN_SCAN_INTERVAL_MS, MAX_SCAN_INTERVAL_MS);
        if self.active {
            configured
        } else {
            configured.max(IDLE_MIN_INTERVAL_MS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScanSchedule;

    #[test]
    fn recognized_choices_activate_polling_and_a_miss_returns_to_idle() {
        let mut schedule = ScanSchedule::new();
        assert!(!schedule.is_active());
        assert_eq!(schedule.interval_ms(900), 1500);

        schedule.observe(true);
        assert!(schedule.is_active());
        assert_eq!(schedule.interval_ms(900), 900);

        schedule.observe(false);
        assert!(!schedule.is_active());
        assert_eq!(schedule.interval_ms(900), 1500);
    }

    #[test]
    fn slower_user_preferences_survive_both_modes() {
        let mut schedule = ScanSchedule::new();
        for configured in [1500, 2500, 5000] {
            schedule.observe(false);
            assert_eq!(schedule.interval_ms(configured), configured);
            schedule.observe(true);
            assert_eq!(schedule.interval_ms(configured), configured);
        }
    }

    #[test]
    fn reset_forgets_a_previous_choice_and_applies_current_preferences() {
        let mut schedule = ScanSchedule::new();
        schedule.observe(true);
        assert_eq!(schedule.interval_ms(400), 400);

        schedule.reset();
        assert!(!schedule.is_active());
        assert_eq!(schedule.interval_ms(400), 1500);
        assert_eq!(schedule.interval_ms(3000), 3000);
    }

    #[test]
    fn invalid_intervals_remain_bounded() {
        let mut schedule = ScanSchedule::new();
        assert_eq!(schedule.interval_ms(0), 1500);
        assert_eq!(schedule.interval_ms(u64::MAX), 5000);

        schedule.observe(true);
        assert_eq!(schedule.interval_ms(0), 400);
        assert_eq!(schedule.interval_ms(u64::MAX), 5000);
    }
}
