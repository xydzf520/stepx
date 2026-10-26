use std::{io::{self, BufRead, Write}, time::Instant};
use stepx_companion::{adb::Adb, device::Device, amoo::AgentBus, bridge};
fn main() -> anyhow::Result<()> {
 let adb=Adb::discover()?;
 let serial=std::env::var("STEPX_BENCH_DEVICE")?;
 let device=Device::inspect(&adb,&serial)?;
 let session=std::env::var("STEPX_BENCH_SESSION")?;
 let mut bus=AgentBus::observe(&adb,&device)?;
 println!("ready"); io::stdout().flush()?;
 for line in io::stdin().lock().lines() {
  let metric=line?;if metric=="quit" {break;}
  let start=Instant::now();
  let result: anyhow::Result<usize> = match metric.as_str() {
   "device_summary"=>device.summary(&adb).map(|v|v.len()),
   "phone_capabilities"=>bridge::capabilities(&adb,&device).map(|v|v.subscriptions.len()),
   "amoo_snapshot"=>bus.snapshot(Some(&session)).map(|v|v.sessions.len()),
   _=>anyhow::bail!("unknown metric")
  };
  let elapsed=start.elapsed().as_secs_f64()*1000.0;
  match result {
   Ok(count)=>println!("{}",serde_json::json!({"metric":metric,"ms":elapsed,"ok":true,"count":count})),
   Err(e)=>println!("{}",serde_json::json!({"metric":metric,"ms":elapsed,"ok":false,"error":e.to_string()}))
  };
  io::stdout().flush()?;
 }
 Ok(())
}
