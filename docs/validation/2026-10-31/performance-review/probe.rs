use modorch::*;
use std::{sync::Arc, time::{Duration, Instant}};
struct Echo;
#[async_trait]
impl SerialHandler for Echo {
    async fn handle(&mut self, _: &HandlerContext, message: MessageEnvelope) -> FrameworkResult<HandlerReply> { Ok(message.payload) }
}
fn rss() -> usize {
    std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|s| s.starts_with("VmRSS:")).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap()
}
fn setup(subscriptions: usize, capacity: usize) -> (EventHub, OperationTracker) {
    let tracker=OperationTracker::new(); tracker.open().unwrap();
    let hub=EventHub::new(tracker.clone(), FailureReporter::new());
    let mut d=ComponentDescriptor::new(ComponentRef::module("worker")); d.queue_capacity=capacity;
    let subscriptions=(0..subscriptions).map(|i| SubscriptionSpec::new(i as u64,d.component(),if i+1==subscriptions { "echo".into() } else { format!("unused/{i}") })).collect();
    hub.install(&d,HandlerRegistration::Serial { handler:Box::new(Echo),subscriptions }).unwrap();
    (hub,tracker)
}
async fn stop(hub: EventHub, tracker: OperationTracker) { tracker.close(); tracker.drain().await; hub.terminate(false).await; }
struct MixedHandler { kind: &'static str, isolated: bool, cpu_slots: Arc<tokio::sync::Semaphore> }
fn cpu_work() {
    let start=Instant::now();let mut value=1u64;
    while start.elapsed()<Duration::from_millis(20) { for _ in 0..128 { value=value.wrapping_mul(6364136223846793005).wrapping_add(1); } std::hint::black_box(value); }
}
#[async_trait]
impl ConcurrentHandler for MixedHandler {
    async fn handle(&self, _: &HandlerContext, message: MessageEnvelope) -> FrameworkResult<HandlerReply> {
        match self.kind {
            "cpu" if self.isolated => {
                let permit=self.cpu_slots.clone().acquire_owned().await.unwrap();
                tokio::task::spawn_blocking(move || { let _permit=permit;cpu_work(); }).await.unwrap();
            }
            "cpu" => cpu_work(),
            "io" => tokio::time::sleep(Duration::from_millis(1)).await,
            _ => {},
        }
        Ok(message.payload)
    }
}
async fn mixed(isolated: bool) {
    let tracker=OperationTracker::new();tracker.open().unwrap();let hub=EventHub::new(tracker.clone(),FailureReporter::new());
    let cpu_slots=Arc::new(tokio::sync::Semaphore::new(2));
    for (i,kind) in ["cpu","io","short"].into_iter().enumerate() {
        let mut d=ComponentDescriptor::new(ComponentRef::module(kind));d.execution=ExecutionPolicy::Concurrent { max_in_flight:std::num::NonZeroUsize::new(4).unwrap() };d.queue_capacity=16;
        hub.install(&d,HandlerRegistration::Concurrent { max_in_flight:std::num::NonZeroUsize::new(4).unwrap(),handler:Arc::new(MixedHandler {kind,isolated,cpu_slots:cpu_slots.clone()}),subscriptions:vec![SubscriptionSpec::new(i as u64,d.component(),"work")] }).unwrap();
    }
    let deadline=Instant::now()+Duration::from_secs(2);let mut joins=tokio::task::JoinSet::new();
    for kind in ["cpu","io","short"] { for _ in 0..4 { let hub=hub.clone();joins.spawn(async move {
        let mut latencies=Vec::new();let mut count=0;
        while Instant::now()<deadline {
            let start=Instant::now();hub.request_wait::<_,u64>("work",Destination::Component(ComponentRef::module(kind)),1u64,Default::default(),Duration::from_secs(5)).await.unwrap().wait(Duration::from_secs(5)).await.unwrap();
            count+=1;if latencies.len()<100000 { latencies.push(start.elapsed().as_micros() as u64); }
        }
        (kind,count,latencies)
    }); } }
    let mut stats=std::collections::BTreeMap::<&str,(usize,Vec<u64>)>::new();
    while let Some(result)=joins.join_next().await { let (kind,count,mut latencies)=result.unwrap();let entry=stats.entry(kind).or_default();entry.0+=count;entry.1.append(&mut latencies); }
    stop(hub,tracker).await;
    let stats:std::collections::BTreeMap<_,_>=stats.into_iter().map(|(kind,(count,mut v))| {v.sort_unstable();let q=|p:usize|v[(v.len()*p).div_ceil(100).saturating_sub(1)];(kind,serde_json::json!({"completed":count,"latency_samples":v.len(),"p50_micros":q(50),"p95_micros":q(95),"p99_micros":q(99),"max_micros":v.last()}))}).collect();
    println!("{}",serde_json::json!({"case":"mixed","isolated":isolated,"tokio_workers":4,"blocking_cpu_limit":if isolated {2} else {0},"cpu_work_millis":20,"io_sleep_millis":1,"seconds_target":2,"stats":stats}));
}
struct SaturatingCpu { isolated:bool, barrier:Arc<std::sync::Barrier> }
fn saturating_work(barrier: Arc<std::sync::Barrier>) {
    barrier.wait();let start=Instant::now();while start.elapsed()<Duration::from_millis(100) { std::hint::black_box(123u64.wrapping_mul(987)); }
}
#[async_trait]
impl ConcurrentHandler for SaturatingCpu {
    async fn handle(&self, _: &HandlerContext, message: MessageEnvelope) -> FrameworkResult<HandlerReply> {
        let barrier=self.barrier.clone();
        if self.isolated { tokio::task::spawn_blocking(move || saturating_work(barrier)).await.unwrap(); }
        else { saturating_work(barrier); }
        Ok(message.payload)
    }
}
async fn saturation(isolated:bool) {
    let tracker=OperationTracker::new();tracker.open().unwrap();let hub=EventHub::new(tracker.clone(),FailureReporter::new());let barrier=Arc::new(std::sync::Barrier::new(5));
    let mut d=ComponentDescriptor::new(ComponentRef::module("cpu"));let n=std::num::NonZeroUsize::new(4).unwrap();d.execution=ExecutionPolicy::Concurrent { max_in_flight:n };
    hub.install(&d,HandlerRegistration::Concurrent {max_in_flight:n,handler:Arc::new(SaturatingCpu {isolated,barrier:barrier.clone()}),subscriptions:vec![SubscriptionSpec::new(1,d.component(),"cpu")] }).unwrap();
    let mut io=ComponentDescriptor::new(ComponentRef::module("io"));io.execution=ExecutionPolicy::Concurrent {max_in_flight:n};
    hub.install(&io,HandlerRegistration::Concurrent { max_in_flight:n,handler:Arc::new(MixedHandler {kind:"io",isolated:false,cpu_slots:Arc::new(tokio::sync::Semaphore::new(1))}),subscriptions:vec![SubscriptionSpec::new(2,io.component(),"io")] }).unwrap();
    let mut requests=Vec::new();for _ in 0..4 {requests.push(hub.request::<_,u64>("cpu",Destination::Component(d.component()),1u64,Default::default()).unwrap());}
    let (tx,rx)=tokio::sync::oneshot::channel();let ingress=hub.clone();
    let ingress_thread=std::thread::spawn(move || {barrier.wait();std::thread::sleep(Duration::from_millis(1));let start=Instant::now();let request=ingress.request::<_,u64>("io",Destination::Component(io.component()),1u64,Default::default()).unwrap();tx.send((start,request)).ok().unwrap();});
    let (start,request)=tokio::time::timeout(Duration::from_secs(10),rx).await.unwrap().unwrap();request.wait(Duration::from_secs(5)).await.unwrap();let latency=start.elapsed();
    for request in requests {request.wait(Duration::from_secs(5)).await.unwrap();}ingress_thread.join().unwrap();stop(hub,tracker).await;
    println!("{}",serde_json::json!({"case":"saturation","isolated":isolated,"tokio_workers":4,"cpu_jobs":4,"cpu_busy_millis":100,"io_sleep_millis":1,"io_latency_micros":latency.as_micros()}));
}
#[tokio::main(worker_threads=4)]
async fn main() {
    let args:Vec<String>=std::env::args().collect();
    match args[1].as_str() {
        "saturation" => saturation(args[2]=="isolated").await,
        "mixed" => mixed(args[2]=="isolated").await,
        "route" => {
            let subscriptions:usize=args[2].parse().unwrap(); let n:usize=args[3].parse().unwrap();
            let (hub,tracker)=setup(subscriptions,256); let begin=Instant::now();
            let mut joins=tokio::task::JoinSet::new();
            for _ in 0..32 { let hub=hub.clone(); joins.spawn(async move { for _ in 0..n { hub.request_wait::<_,u64>("echo",Destination::Component(ComponentRef::module("worker")),1u64,Default::default(),Duration::from_secs(5)).await.unwrap().wait(Duration::from_secs(5)).await.unwrap(); } }); }
            while let Some(result)=joins.join_next().await { result.unwrap(); }
            let elapsed=begin.elapsed();stop(hub,tracker).await;
            println!("{}",serde_json::json!({"case":"route","subscriptions":subscriptions,"requests":32*n,"seconds":elapsed.as_secs_f64(),"requests_per_second":(32*n) as f64/elapsed.as_secs_f64()}));
        }
        "observation" => {
            let (hub,tracker)=setup(1,8); let before=rss();
            let bytes=Arc::new(vec![7u8;16*1024*1024]); let weak=Arc::downgrade(&bytes);
            let request=hub.request::<_,Arc<Vec<u8>>>("echo",Destination::Component(ComponentRef::module("worker")),bytes,Default::default()).unwrap();
            let observation=request.observation(); let reply=request.wait(Duration::from_secs(5)).await.unwrap();drop(reply);
            stop(hub,tracker).await;
            let retained=weak.strong_count()>0; let after_reply=rss();
            drop(observation); let released=weak.strong_count()==0; let after_observer=rss();
            println!("{}",serde_json::json!({"case":"observation","bytes":16*1024*1024,"retained_after_reply_consumed":retained,"released_after_observer_drop":released,"rss_before_kib":before,"rss_after_reply_kib":after_reply,"rss_after_observer_drop_kib":after_observer}));
        }
        _ => panic!("unknown case")
    }
}
