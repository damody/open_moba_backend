//! Bloons 2.5 upgrade rule validator.
//!
//! 對任一 tower 的 `[u8; 3]` path levels，要求升第 i 條路線：
//! - 主路線（level ≥ 3）最多 1 條
//! - 副路線（1 ≤ level ≤ 2）最多 1 條
//! - 第三路線必須 0

#[derive(Debug, PartialEq, Eq)]
pub enum UpgradeRejection {
    AlreadyMaxed,
    TwoPrimaryPaths,
    TwoSecondaryPaths,
    ThirdPathLocked,
}

pub fn validate_upgrade(levels: [u8; 3], path: u8) -> Result<(), UpgradeRejection> {
    if path >= 3 {
        return Err(UpgradeRejection::ThirdPathLocked);
    }
    let i = path as usize;
    if levels[i] >= 4 {
        return Err(UpgradeRejection::AlreadyMaxed);
    }
    let mut next = levels;
    next[i] += 1;

    let primary = next.iter().filter(|&&l| l >= 3).count();
    let secondary = next.iter().filter(|&&l| l >= 1 && l <= 2).count();

    if primary > 1 {
        return Err(UpgradeRejection::TwoPrimaryPaths);
    }
    if primary == 1 && secondary > 1 {
        return Err(UpgradeRejection::TwoSecondaryPaths);
    }
    if primary == 0 && secondary > 2 {
        return Err(UpgradeRejection::TwoSecondaryPaths);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn empty_any_ok() {
        assert!(validate_upgrade([0,0,0], 0).is_ok());
        assert!(validate_upgrade([0,0,0], 1).is_ok());
        assert!(validate_upgrade([0,0,0], 2).is_ok());
    }

    #[test] fn max_rejected() {
        assert_eq!(validate_upgrade([4,0,0], 0), Err(UpgradeRejection::AlreadyMaxed));
    }

    #[test] fn two_primary_rejected() {
        // Path 0 L3 primary，升 Path 1 到 L3 會違反
        assert_eq!(validate_upgrade([3,2,0], 1), Err(UpgradeRejection::TwoPrimaryPaths));
    }

    #[test] fn two_secondary_when_primary() {
        // Path 0 L3 primary，Path 1 L1 secondary，要把 Path 2 升 → 會變 2 個 secondary
        assert_eq!(validate_upgrade([3,1,0], 2), Err(UpgradeRejection::TwoSecondaryPaths));
    }

    #[test] fn three_secondary_no_primary() {
        // 無主路線時不能三條都升
        assert_eq!(validate_upgrade([2,1,0], 2), Err(UpgradeRejection::TwoSecondaryPaths));
    }

    #[test] fn path_upgrade_to_primary_ok() {
        // Path 0 L2 → L3（升主），Path 1 L2 副 — 合法
        assert!(validate_upgrade([2,2,0], 0).is_ok());
    }

    #[test] fn full_build_ok() {
        // 主 L4 + 副 L2 能達成
        assert!(validate_upgrade([3,2,0], 0).is_ok());  // 升主 L4
    }
}
