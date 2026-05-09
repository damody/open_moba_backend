use crate::comp::base::span;
use specs::{ReadExpect, RunNow};
use std::sync::Mutex;
use std::{collections::HashMap, time::Instant};

#[derive(Default)]
pub struct SysMetrics {
    pub stats: Mutex<HashMap<String, CpuTimeline>>,
}

/// 測量程式碼單元運行的執行緒級別。運行時使用 Rayon
/// 在他們的線程池上。當您知道程式碼有多少個執行緒時使用 Exact
/// 準確地跑下去。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ParMode {
    None, /* Job is not running at all */
    Single,
    Rayon,
    Exact(u32),
}

#[derive(Default, Debug, Clone)]
pub struct CpuTimeline {
    /// 系統測量
    /// - 第一個條目將始終是 ParMode::Single，就像當
    /// System::run執行完畢，我們運行
    /// 單線程，直到我們啟動 Rayon::ParIter 或類似的
    /// - 最後一個條目將包含系統的結束時間。來標記
    /// 結束它總是包含
    /// ParMode::None，這意味著從那時起有 0 個 CPU 執行緒在此工作
    /// 系統
    measures: Vec<(Instant, ParMode)>,
}

#[derive(Default)]
pub struct CpuTimeStats {
    /// 第一個條目始終為 0，最後一個條目始終為“dt”
    /// 從“ns”開始的“usage”
    measures: Vec<(/* ns */ u64, /* usage */ f32)>,
}

/// 並行模式告訴我們您的擴充程度。 「無」表示您的程式碼
/// 沒有運行。 「單」表示您正在運行單執行緒。
/// `Rayon` 表示您正在 rayon 執行緒池上執行。
impl ParMode {
    fn threads(&self, rayon_threads: u32) -> u32 {
        match self {
            ParMode::None => 0,
            ParMode::Single => 1,
            ParMode::Rayon => rayon_threads,
            ParMode::Exact(u) => *u,
        }
    }
}

impl CpuTimeline {
    fn reset(&mut self) {
        self.measures.clear();
        self.measures.push((Instant::now(), ParMode::Single));
    }

    /// 開始新的測量。 par will be covering the parallelisation AFTER
    /// 這條語句，直到系統的下一個/結束。
    pub fn measure(&mut self, par: ParMode) {
        self.measures.push((Instant::now(), par));
    }

    fn end(&mut self) -> std::time::Duration {
        let end = Instant::now();
        self.measures.push((end, ParMode::None));
        end.duration_since(
            self.measures
                .first()
                .expect("We just pushed onto the vector.")
                .0,
        )
    }

    fn get(&self, time: Instant) -> ParMode {
        match self.measures.binary_search_by_key(&time, |&(a, _)| a) {
            Ok(id) => self.measures[id].1,
            Err(0) => ParMode::None, /* not yet started */
            Err(id) => self.measures[id - 1].1,
        }
    }
}

impl CpuTimeStats {
    pub fn length_ns(&self) -> u64 {
        self.end_ns() - self.start_ns()
    }

    pub fn start_ns(&self) -> u64 {
        self.measures
            .iter()
            .find(|e| e.1 > 0.001)
            .unwrap_or(&(0, 0.0))
            .0
    }

    pub fn end_ns(&self) -> u64 {
        self.measures.last().unwrap_or(&(0, 0.0)).0
    }

    pub fn avg_threads(&self) -> f32 {
        let mut sum = 0.0;
        for w in self.measures.windows(2) {
            let len = w[1].0 - w[0].0;
            let h = w[0].1;
            sum += len as f32 * h;
        }
        sum / (self.length_ns() as f32)
    }
}

/// 這個想法是將每個系統的單獨時間線轉換為所有系統的地圖
/// 核心以及它們（可能）正在做什麼。
///
/// ＃ 例子
///
/// - 輸入：3 個服務，0 和 1 是 100% 並行，2 是單執行緒。 ``-``
/// 意味著 *0.5 秒* 內不工作。 `#` 表示*0.5s* 的完整工作。我們看到第一個
/// 服務在 1 秒後啟動並運行 3 秒第二個服務在一秒後啟動
/// 並運轉 4 秒。最後一個服務在tick啟動後運行2.5s並運行
/// 1秒。從左到右閱讀。
/// 『忽略
/// [--######------]
/// [----########--]
/// [-----##-------]
/// ```
///
/// - 產出：一張地圖，計算我們的 6 個核心將時間花在哪裡。
/// 這裡每個數字表示 50% 的核心正在處理它。 「-」代表一個
/// 空轉核心。我們從所有 6 個核心都處於空閒狀態開始。然後所有核心開始
/// 處理任務 0。2 秒後，任務 1 啟動，我們必須拆分核心。 2.5秒內
/// 任務2開始。我們有 6 個實體線程，但需要填充 13 個。稍後的任務 2
/// 任務 0 將完成其工作並為任務 1 提供更多執行緒來工作
/// 在。從上到下閱讀
/// 『忽略
/// 0-1s     [------------]
/// 1-2s     [000000000000]
/// 2-2.5s   [000000111111]
/// 2.5-3.5s [000001111122]
/// 3.5-4s   [000000111111]
/// 4-6s     [111111111111]
/// 6s..     [------------]
/// ```
pub fn gen_stats(
    timelines: &HashMap<String, CpuTimeline>,
    tick_work_start: Instant,
    rayon_threads: u32,
    physical_threads: u32,
) -> HashMap<String, CpuTimeStats> {
    let mut result = HashMap::new();
    let mut all = timelines
        .iter()
        .flat_map(|(s, t)| {
            let mut stat = CpuTimeStats::default();
            stat.measures.push((0, 0.0));
            result.insert(s.clone(), stat);
            t.measures.iter().map(|e| &e.0)
        })
        .collect::<Vec<_>>();

    all.sort();
    all.dedup();
    for time in all {
        let relative_time = time.duration_since(tick_work_start).as_nanos() as u64;
        // 在這個特定時間獲得所有並行化
        let individual_cores_wanted = timelines
            .iter()
            .map(|(k, t)| (k, t.get(*time).threads(rayon_threads)))
            .collect::<Vec<_>>();
        let total = individual_cores_wanted
            .iter()
            .map(|(_, a)| a)
            .sum::<u32>()
            .max(1) as f32;
        let total_or_max = total.max(physical_threads as f32);
        // 更新所有狀態
        for individual in individual_cores_wanted.iter() {
            let actual = (individual.1 as f32 / total_or_max) * physical_threads as f32;
            if let Some(p) = result.get_mut(individual.0) {
                if p.measures
                    .last()
                    .map(|last| (last.1 - actual).abs())
                    .unwrap_or(0.0)
                    > 0.0001
                {
                    p.measures.push((relative_time, actual));
                }
            } else {
                tracing::warn!("Invariant violation: keys in both hashmaps should be the same.");
            }
        }
    }
    result
}

/// 此特徵圍繞著specs::System 並進行額外的指標收集
///
/// ```
/// 使用規範::閱讀；
/// 使用 omobab::comp::ecs::{作業、ParMode、系統}；
/// # 使用 std::time::Duration；
/// pub 結構系統；
/// impl<'a> System<'a> for Sys {
/// 類型 SystemData = (Read<'a, ()>, Read<'a, ()>);
///
/// const NAME: &'static str = "範例";
///
/// fn run(作業: &mut Job<Self>, (_read, _read2): Self::SystemData) {
///         std::thread::sleep(Duration::from_millis(100));
///         job.cpu_stats.measure(ParMode::Rayon);
///         std::thread::sleep(Duration::from_millis(500));
///         job.cpu_stats.measure(ParMode::Single);
///         std::thread::sleep(Duration::from_millis(40));
///     }
/// }
/// ```
pub trait System<'a> {
    const NAME: &'static str;

    type SystemData: specs::SystemData<'a>;
    fn run(job: &mut Job<Self>, data: Self::SystemData);
    fn sys_name() -> String {
        format!("{}_sys", Self::NAME)
    }
}

pub fn dispatch<'a, 'b, T>(builder: &mut specs::DispatcherBuilder<'a, 'b>, dep: &[&str])
where
    T: for<'c> System<'c> + Send + 'a + Default,
{
    builder.add(Job::<T>::default(), &T::sys_name(), dep);
}

pub fn run_now<'a, 'b, T>(world: &'a specs::World)
where
    T: for<'c> System<'c> + Send + 'a + Default,
{
    Job::<T>::default().run_now(world);
}

/// 該結構將包裝系統以避免 can only impl 特徵
/// 對於本地定義的結構錯誤它還包含 cpu 測量
pub struct Job<T>
where
    T: ?Sized,
{
    pub own: Box<T>,
    pub cpu_stats: CpuTimeline,
}

impl<'a, T> specs::System<'a> for Job<T>
where
    T: System<'a>,
{
    type SystemData = (
        T::SystemData,
        ReadExpect<'a, SysMetrics>,
        ReadExpect<'a, crate::comp::TickProfile>,
    );

    fn run(&mut self, data: Self::SystemData) {
        // 舊版每次 run 會做三件浪費：
        //   1. format!("{}::Sys::run", T::NAME) alloc（每 tick × 13 systems）
        //   2. cpu_stats.reset/end 的 Vec<(Instant, ParMode)> 分配
        //   3. SysMetrics.stats.lock().insert(String::new) — Mutex 把 par_join 排序化
        // SysMetrics HashMap 目前無人讀，全部砍掉；若之後要重啟 profiling，
        // 在這裡改用 lock-free per-thread 統計即可。
        let start = std::time::Instant::now();
        T::run(self, data.0);
        let elapsed = start.elapsed();
        let ns = elapsed.as_nanos();
        // ReadExpect<TickProfile> 拿到 &TickProfile，內部 Mutex 處理並行寫入。
        data.2.record_system(T::NAME, ns);
        let millis = elapsed.as_millis();
        if millis > 500 {
            let name = T::NAME;
            tracing::warn!(?millis, ?name, "slow system execution");
        }
    }
}

impl<'a, T> Default for Job<T>
where
    T: System<'a> + Default,
{
    fn default() -> Self {
        Self {
            own: Box::new(T::default()),
            cpu_stats: CpuTimeline::default(),
        }
    }
}
