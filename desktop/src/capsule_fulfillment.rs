//! Local fulfillment evidence for the desktop capsule. No app or model actions.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use crate::home_hub::{
    classify_delivery_or_mobility_signal, extract_express_from_text_flexible,
    is_completed_express_text, is_synthetic_e2e_probe_text,
};

pub const DEFAULT_AGENT_PROMPT: &str = "判断物流提醒的对象。只有短语能证明包裹送给当前用户时才判 receive。寄件人关注运输进度、对象不明或证据矛盾时判 unknown。单凭品牌、派件或「您的快件」，判 unknown。";
pub const MAX_PROMPT_CHARS: usize = 800;
pub const ARRIVAL_TTL_MS: i64 = 10 * 60_000;
const EVIDENCE_WINDOW_MS: i64 = 72 * 3_600_000;
const WEAK_WINDOW_MS: i64 = 45 * 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Parcel,
    Instant,
    Travel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, serde::Serialize)]
pub enum Stage {
    Accepted,
    Moving,
    Ready,
    Arrived,
    Finished,
}

impl Stage {
    pub fn label(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Self::Accepted, _) => "已接单",
            (Self::Moving, Kind::Parcel) => "物流在途",
            (Self::Moving, Kind::Travel) => "行程中",
            (Self::Moving, Kind::Instant) => "配送中",
            (Self::Ready, Kind::Parcel) => "待取件",
            (Self::Ready, _) => "待取餐",
            (Self::Arrived, _) => "已送达",
            (Self::Finished, _) => "已完成",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Receive,
    Send,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Sms(String),
    Notification(String),
    Thread(String),
}

#[derive(Clone, Debug)]
pub struct Signal {
    pub provider: String,
    pub title: String,
    pub detail: String,
    pub tracking: Option<String>,
    pub order_suffix: Option<String>,
    pub kind: Kind,
    pub stage: Stage,
    pub direction: Direction,
    pub dispatching: bool,
    pub at: i64,
    pub origin: Origin,
    pub seen: bool,
    pub id: String,
    /// Hash of the provider and order reference when one exists.
    pub(crate) strong: bool,
    pub cues: Vec<&'static str>,
}

impl Signal {
    pub fn current(&self, now: i64, normal_ttl: i64) -> bool {
        let ttl = if self.stage == Stage::Arrived {
            normal_ttl.min(ARRIVAL_TTL_MS)
        } else {
            normal_ttl
        };
        self.at <= now
            && now.saturating_sub(self.at) < ttl
            && self.stage != Stage::Finished
            && !(self.stage == Stage::Arrived && self.seen)
    }

    pub fn agent_candidate(&self) -> bool {
        self.kind == Kind::Parcel
            && self.direction == Direction::Unknown
            && self.stage < Stage::Arrived
            && self.cues.iter().any(|cue| specific_recipient_cue(cue))
    }

    pub fn agent_key(&self) -> String {
        digest(&[&self.provider, &self.cues.join("|")])
    }
}

pub fn digest(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    format!("{:x}", h.finalize())[..24].to_string()
}

fn has(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

const SEND: &[&str] = &[
    "您寄出的",
    "您寄出",
    "您寄递",
    "您寄送",
    "您已寄",
    "您发出的",
    "您发给",
    "您下单寄件",
    "您的寄件",
    "为您上门取件",
    "上门揽收",
    "寄件订单",
    "寄件成功",
    "已揽收您",
    "寄件人是您",
    "您的寄件人",
    "您寄的",
    "您发的",
    "寄出包裹",
    "寄件进度",
];
const RECEIVE: &[&str] = &[
    "寄给您",
    "收件人是您",
    "您是收件人",
    "您的收件包裹",
    "您的待收",
    "待您签收",
    "请您签收",
    "请您查收",
    "请保持电话畅通以便签收",
    "请您取件",
    "请凭取件码",
    "凭码取件",
    "请及时取件",
    "您的收货地址",
    "等待您签收",
    "您购买的商品",
];
const SOFT_RECEIVE: &[&str] = &[
    "投递至您处",
    "送至您处",
    "送到您手中",
    "交到您手中",
    "您留存的收件信息",
    "您选择的收货点",
    "将由您接收",
    "为您投递",
    "您的快件",
    "快件正在派送",
];

fn specific_recipient_cue(cue: &str) -> bool {
    !matches!(cue, "您的快件" | "快件正在派送" | "为您投递")
}

pub fn parcel_direction(text: &str) -> Direction {
    if has(text, SEND) {
        Direction::Send
    } else if has(text, RECEIVE) {
        Direction::Receive
    } else {
        Direction::Unknown
    }
}

fn order_reference(text: &str) -> Option<String> {
    for label in ["订单编号", "订单号", "订单尾号", "尾号"] {
        let Some((_, rest)) = text.split_once(label) else {
            continue;
        };
        let value: String = rest
            .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '：' | ':' | '#' | '【'))
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(40)
            .collect();
        if value.len() >= 4 && value.bytes().filter(u8::is_ascii_digit).count() >= 4 {
            return Some(format!(
                "{}{}",
                if label.contains("尾号") {
                    "tail:"
                } else {
                    "order:"
                },
                value
            ));
        }
    }
    None
}

pub fn classify(
    pkg: &str,
    app: &str,
    title: &str,
    body: &str,
    origin: Origin,
    at: i64,
    seen: bool,
    now: i64,
) -> Option<Signal> {
    if at <= 0
        || at > now
        || now.saturating_sub(at) > EVIDENCE_WINDOW_MS
        || is_synthetic_e2e_probe_text(body)
    {
        return None;
    }
    let text = format!("{title} {body}");
    if has(
        &text,
        &[
            "优惠券",
            "领券",
            "专享优惠",
            "配送优惠",
            "免配送费",
            "积分到账",
            "活动通知",
            "验证码",
        ],
    ) && !has(&text, &["订单号", "订单尾号", "运单", "快件已", "商品已"])
    {
        return None;
    }
    let source = format!("{app} {title}");
    let prefix = body
        .trim_start()
        .strip_prefix('【')
        .and_then(|body| body.split_once('】'))
        .map_or("", |(prefix, _)| prefix);
    let service = format!("{source} {prefix}");
    let carriers = [
        ("顺丰", "顺丰速运"),
        ("顺风快递", "顺丰速运"),
        ("中通", "中通快递"),
        ("圆通", "圆通速递"),
        ("申通", "申通快递"),
        ("韵达", "韵达快递"),
        ("极兔", "极兔速递"),
        ("京东快递", "京东快递"),
        ("京东物流", "京东快递"),
        ("EMS", "邮政EMS"),
        ("菜鸟", "菜鸟"),
        ("丰巢", "丰巢"),
    ];
    let carrier = carriers
        .iter()
        .find(|(name, _)| {
            service.contains(name)
                || matches!(origin, Origin::Sms(_)) && body.trim_start().starts_with(name)
        })
        .map(|(_, label)| *label)
        .or_else(|| {
            (pkg == "com.jingdong.app.mall" && has(&text, &["京东物流", "京东快递", "运单号"]))
                .then_some("京东快递")
        });
    let courier_app = has(
        pkg,
        &[
            "com.sf.",
            "com.zto.",
            "com.yto.",
            "com.sto.",
            "com.yunda.",
            "com.cainiao.",
            "com.fcbox.",
        ],
    );
    let messenger = matches!(pkg, "com.tencent.mm" | "com.tencent.mobileqq");
    let parcel = (carrier.is_some() || courier_app)
        && (!messenger
            || carriers
                .iter()
                .any(|(name, _)| title.trim().starts_with(name)))
        && has(
            &text,
            &[
                "运单",
                "快件",
                "快递",
                "包裹",
                "寄件",
                "揽收",
                "派件",
                "取件码",
            ],
        );
    let direction = if parcel {
        parcel_direction(&text)
    } else {
        Direction::Receive
    };
    let finished = has(
        &text,
        &[
            "订单已完成",
            "订单完成",
            "交易完成",
            "行程已结束",
            "行程结束",
            "已下车",
            "已取消",
            "取消成功",
            "退款成功",
            "已退款",
            "已退回",
            "已退件",
            "拒收成功",
            "本人已签收",
            "收件人已签收",
            "已拿取",
            "商品已取",
            "已取餐",
        ],
    ) || (parcel
        && is_completed_express_text(&text)
        && !has(&text, &["已完成揽收", "已完成中转", "已完成分拣"]));
    let ready = has(
        &text,
        &[
            "待取件",
            "取件码",
            "待取餐",
            "取餐码",
            "取茶码",
            "请凭取货码",
            "请取餐",
            "请及时取",
            "可取餐",
            "制作完成",
            "驿站已签收",
            "柜机已签收",
        ],
    );
    let arrived = has(
        &text,
        &[
            "已送达",
            "已放家门口",
            "已放门口",
            "已放在",
            "已代收",
            "送达成功",
            "已投递",
        ],
    ) && !has(&text, &["未送达", "如已送达", "如果已送达", "若已送达"]);
    let dispatching = has(
        &text,
        &[
            "派送中",
            "派件中",
            "正在派送",
            "正在派件",
            "为您派件",
            "为您派送",
        ],
    );
    let moving = dispatching
        || has(
            &text,
            &[
                "配送中",
                "正在配送",
                "骑手正",
                "已取货",
                "已发货",
                "运输中",
                "已揽收",
                "揽收发出",
                "预计送达",
                "预计到达",
                "距您",
                "发往",
                "中转",
                "司机已接单",
                "已上车",
                "检票",
                "候车",
                "登机",
                "起飞",
            ],
        );
    let accepted = has(&text, &["已接单", "正在打包", "备餐", "分拣中", "制作中"]);
    let stage = if finished {
        Stage::Finished
    } else if arrived && !(parcel && ready && crate::home_hub::extract_pickup_code(&text).is_some())
    {
        Stage::Arrived
    } else if ready {
        Stage::Ready
    } else if moving {
        Stage::Moving
    } else if accepted {
        Stage::Accepted
    } else {
        return None;
    };

    let waybill = parcel
        .then(|| {
            extract_express_from_text_flexible(body, has(body, &["运单", "快递单号"])).or_else(
                || extract_express_from_text_flexible(title, has(title, &["运单", "快递单号"])),
            )
        })
        .flatten();
    let tracking = waybill.as_ref().map(|(_, number)| number.clone());
    let (provider, headline, kind) = if parcel {
        let provider = carrier
            .or_else(|| waybill.as_ref().map(|(name, _)| *name))
            .unwrap_or("快递");
        (
            provider.to_string(),
            format!("{provider} · 收件进度"),
            Kind::Parcel,
        )
    } else {
        // Only business-shaped sources can create fulfillment cards. Ordinary
        // conversation or group content saying “到家”“行程” stays in Messages.
        let known_service = has(
            pkg,
            &[
                "com.yaya.zone",
                "com.wudaokou.hippo",
                "cn.samsclub.app",
                "com.pupumall.customer",
                "sankuai.meituan",
                "dianping",
                "me.ele",
                "com.jd.jddj",
                "didi",
                "12306",
                "umetrip",
                "veryzhun",
                "com.lucky.luckyclient",
                "com.heytea.android",
                "com.chagee.app",
            ],
        ) || has(
            &format!("{app} {title}"),
            &[
                "叮咚买菜",
                "盒马",
                "山姆",
                "朴朴",
                "美团",
                "大众点评",
                "小象超市",
                "歪马送酒",
                "饿了么",
                "闪购",
                "瑞幸",
                "喜茶",
                "霸王茶姬",
                "星巴克",
                "12306",
                "航旅纵横",
                "飞常准",
                "滴滴",
            ],
        ) || !messenger
            && body.trim_start().starts_with('【')
            && has(
                body.split('】').next().unwrap_or(""),
                &[
                    "叮咚买菜",
                    "盒马",
                    "山姆",
                    "朴朴",
                    "美团",
                    "大众点评",
                    "小象超市",
                    "歪马送酒",
                    "饿了么",
                    "瑞幸",
                    "喜茶",
                    "霸王茶姬",
                    "星巴克",
                    "12306",
                    "航旅纵横",
                    "飞常准",
                    "滴滴",
                ],
            );
        if !known_service {
            return None;
        }
        let (provider, headline, _, _) =
            classify_delivery_or_mobility_signal(pkg, app, title, body).or_else(|| {
                // A cancellation can lack delivery vocabulary. Reuse brand recognition
                // with a status marker, while keeping the original terminal evidence.
                if stage != Stage::Finished {
                    return None;
                }
                let brand_hint = format!("{} 已送达", app);
                classify_delivery_or_mobility_signal(pkg, app, &brand_hint, "").or_else(|| {
                    let prefix = body.split('】').next().unwrap_or("");
                    classify_delivery_or_mobility_signal(pkg, app, &format!("{prefix} 已送达"), "")
                })
            })?;
        let travel = has(&provider, &["出行", "航班", "高铁"]);
        (
            provider,
            headline,
            if travel { Kind::Travel } else { Kind::Instant },
        )
    };
    let order_reference = order_reference(&text);
    let order_suffix = tracking
        .as_deref()
        .or_else(|| {
            order_reference
                .as_deref()
                .and_then(|r| r.split_once(':').map(|(_, suffix)| suffix))
        })
        .and_then(|r| r.get(r.len().saturating_sub(4)..))
        .map(str::to_owned);
    let reference = tracking.clone().or(order_reference).map(|reference| {
        if reference.starts_with("tail:") {
            // A short suffix can be reused by a merchant on another day.
            format!("{reference}:{}", at / 86_400_000)
        } else {
            reference
        }
    });
    let body_text = body
        .trim()
        .split_once('】')
        .map(|(_, rest)| rest.trim())
        .unwrap_or(body.trim());
    let identity = if let Some(reference) = &reference {
        if parcel {
            digest(&["parcel", reference])
        } else {
            digest(&[&provider, reference])
        }
    } else {
        // Identical mirrored messages can share an identity, but brand alone
        // never suppresses an independent order from another source.
        let content = if body_text.is_empty() {
            title
        } else {
            body_text
        };
        digest(&[
            &provider,
            content.trim(),
            &(at / WEAK_WINDOW_MS).to_string(),
        ])
    };
    let cues = SOFT_RECEIVE
        .iter()
        .copied()
        .filter(|cue| {
            parcel
                && !messenger
                && text
                    .split(['，', ',', '。', '；', ';', '！', '!', '？', '?'])
                    .any(|clause| {
                        clause.contains(cue)
                            && !has(
                                clause,
                                &[
                                    "无法", "不能", "未能", "未送", "暂不", "能否", "是否", "如需",
                                    "如果",
                                ],
                            )
                    })
        })
        .collect();
    Some(Signal {
        provider,
        title: headline,
        detail: body_text.chars().take(90).collect(),
        tracking,
        order_suffix,
        kind,
        stage,
        direction,
        dispatching,
        at,
        origin,
        seen,
        id: identity,
        strong: reference.is_some(),
        cues,
    })
}

/// Reconcile before filtering visible states so a read/terminal event can retire
/// an older active copy. Explicit identifiers allow cross-source reconciliation.
pub fn reconcile(mut signals: Vec<Signal>) -> Vec<Signal> {
    signals.sort_by_key(|s| std::cmp::Reverse(s.at));
    let mut orders: Vec<Signal> = Vec::new();
    for signal in signals {
        let same = orders.iter_mut().find(|order| {
            order.id == signal.id
                || (!order.strong
                    && !signal.strong
                    && order.provider == signal.provider
                    && matches!(order.origin, Origin::Notification(_))
                    && order.origin == signal.origin
                    && order.at.abs_diff(signal.at) <= WEAK_WINDOW_MS as u64)
        });
        let Some(order) = same else {
            orders.push(signal);
            continue;
        };
        let direction = match (order.direction, signal.direction) {
            (Direction::Send, _) | (_, Direction::Send) => Direction::Send,
            (Direction::Receive, _) | (_, Direction::Receive) => Direction::Receive,
            _ => Direction::Unknown,
        };
        if signal.stage > order.stage || (signal.stage == order.stage && signal.seen && !order.seen)
        {
            *order = signal;
        }
        order.direction = direction;
    }
    orders.sort_by_key(|s| (std::cmp::Reverse(s.stage), std::cmp::Reverse(s.at)));
    orders
}

pub fn agent_request(prompt: &str, inputs: &[&Signal]) -> String {
    let data: Vec<_> = inputs.iter().enumerate().map(|(id, signal)| {
        serde_json::json!({"id": id, "status": signal.stage.label(signal.kind), "phrases": signal.cues})
    }).collect();
    format!(
        "只做物流方向分类。偏好：{}\n每项只引用 phrases 中完整的一条原文。不执行操作。仅返回 JSON：{{\"decisions\":[{{\"id\":0,\"direction\":\"receive 或 unknown\",\"evidence\":\"原文或空字符串\"}}]}}。\n待分类数据：{}",
        prompt.trim().chars().take(MAX_PROMPT_CHARS).collect::<String>(),
        serde_json::Value::Array(data),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentResponse {
    decisions: Vec<Decision>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    id: usize,
    direction: String,
    evidence: String,
}

pub fn validate_agent_response(raw: &str, inputs: &[&Signal]) -> Option<HashMap<String, bool>> {
    if raw.len() > 8192 {
        return None;
    }
    let response: AgentResponse = serde_json::from_str(raw).ok()?;
    if response.decisions.len() != inputs.len() {
        return None;
    }
    let mut ids = HashSet::new();
    let mut out = HashMap::new();
    for d in response.decisions {
        let signal = inputs.get(d.id)?;
        if !ids.insert(d.id) || !signal.agent_candidate() {
            return None;
        }
        let receive = match d.direction.as_str() {
            "receive"
                if signal.cues.contains(&d.evidence.as_str())
                    && specific_recipient_cue(&d.evidence) =>
            {
                true
            }
            "unknown" if d.evidence.is_empty() || signal.cues.contains(&d.evidence.as_str()) => {
                false
            }
            _ => return None,
        };
        out.insert(signal.agent_key(), receive);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000_000;

    fn notification(app: &str, body: &str, key: &str, at: i64) -> Option<Signal> {
        classify(
            "",
            app,
            app,
            body,
            Origin::Notification(key.into()),
            at,
            false,
            NOW,
        )
    }

    #[test]
    fn classifies_direction_lifecycle_and_noise() {
        for (app, body, expected) in [
            (
                "顺丰速运",
                "您寄出的快件 SF123456789012 正在派件，请提醒收件人签收",
                Some((Kind::Parcel, Stage::Moving, Direction::Send)),
            ),
            (
                "顺丰速运",
                "您的快件 SF123456789012 正在派件",
                Some((Kind::Parcel, Stage::Moving, Direction::Unknown)),
            ),
            (
                "顺丰速运",
                "快件 SF223456789012 正在派件，请您签收",
                Some((Kind::Parcel, Stage::Moving, Direction::Receive)),
            ),
            (
                "顺丰速运",
                "您寄出的快件 SF123456789012 已完成分拣，正在派送",
                Some((Kind::Parcel, Stage::Moving, Direction::Send)),
            ),
            (
                "丰巢",
                "您的包裹已放入快递柜，请凭取件码 1-2-3456 取件，如已取件请忽略",
                Some((Kind::Parcel, Stage::Ready, Direction::Receive)),
            ),
            (
                "顺丰速运",
                "快件 SF223456789012 尚未送达，正在派送",
                Some((Kind::Parcel, Stage::Moving, Direction::Unknown)),
            ),
            (
                "顺丰速运",
                "快件 SF223456789012 已放门口，请您查收",
                Some((Kind::Parcel, Stage::Arrived, Direction::Receive)),
            ),
            (
                "叮咚买菜",
                "订单号 10012345 已送达，商品放在门口",
                Some((Kind::Instant, Stage::Arrived, Direction::Receive)),
            ),
            (
                "叮咚买菜",
                "订单号 10012345 已取消，退款成功",
                Some((Kind::Instant, Stage::Finished, Direction::Receive)),
            ),
            (
                "叮咚买菜",
                "预计送达 19:30，订单号 10022345",
                Some((Kind::Instant, Stage::Moving, Direction::Receive)),
            ),
            ("叮咚买菜", "配送优惠券到账", None),
            (
                "饿了么",
                "订单号 60012345 骑手正在配送",
                Some((Kind::Instant, Stage::Moving, Direction::Receive)),
            ),
            (
                "瑞幸",
                "订单号 70012345 制作完成，取餐码 2185",
                Some((Kind::Instant, Stage::Ready, Direction::Receive)),
            ),
            (
                "滴滴",
                "行程已结束，订单号 80012345",
                Some((Kind::Travel, Stage::Finished, Direction::Receive)),
            ),
            (
                "铁路12306",
                "G104 开始检票",
                Some((Kind::Travel, Stage::Moving, Direction::Receive)),
            ),
            ("微信", "朋友说顺丰快件正在派送，请您签收", None),
            ("微信", "快递单号 SF223456789012 已派件", None),
        ] {
            let got =
                notification(app, body, "fixture", NOW).map(|s| (s.kind, s.stage, s.direction));
            assert_eq!(got, expected, "{app} {body}");
        }
        assert!(classify(
            "com.tencent.mm",
            "微信",
            "同事",
            "【顺丰速运】快件正在派送，送至您处",
            Origin::Notification("forwarded".into()),
            NOW,
            false,
            NOW,
        )
        .is_none());
    }

    #[test]
    fn reconciles_cross_source_orders_without_regressing_or_merging_same_merchant() {
        let older =
            notification("叮咚买菜", "订单号 10012345 正在配送", "a", NOW - 90_000).unwrap();
        let delivered = classify(
            "",
            "",
            "106900",
            "【叮咚买菜】订单号 10012345 已送达",
            Origin::Sms("106900".into()),
            NOW - 60_000,
            false,
            NOW,
        )
        .unwrap();
        let stale_retry =
            notification("叮咚买菜", "订单号 10012345 正在配送", "b", NOW - 30_000).unwrap();
        let another = notification("叮咚买菜", "订单号 20012345 正在配送", "c", NOW).unwrap();
        let result = reconcile(vec![older, delivered.clone(), stale_retry, another.clone()]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, delivered.id);
        assert_eq!(result[0].stage, Stage::Arrived);
        assert_eq!(result[1].id, another.id);
        assert!(delivered.current(delivered.at + ARRIVAL_TTL_MS - 1, 3_600_000));
        assert!(!delivered.current(delivered.at + ARRIVAL_TTL_MS, 3_600_000));

        let mut read_copy = delivered.clone();
        read_copy.seen = true;
        assert!(!reconcile(vec![delivered, read_copy])[0].current(NOW, 3_600_000));
    }

    #[test]
    fn parcel_direction_is_bound_to_waybill_and_outgoing_wins() {
        let sent = notification(
            "顺丰速运",
            "您寄出的快件 SF123456789012 已揽收",
            "a",
            NOW - 60_000,
        )
        .unwrap();
        let progress = notification(
            "顺丰速运",
            "快件 SF123456789012 正在派件，请您签收",
            "b",
            NOW - 30_000,
        )
        .unwrap();
        let inbound = notification(
            "顺丰速运",
            "快件 SF223456789012 正在派件，请您签收",
            "c",
            NOW,
        )
        .unwrap();
        let result = reconcile(vec![progress, sent.clone(), inbound.clone()]);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.iter().find(|s| s.id == sent.id).unwrap().direction,
            Direction::Send
        );
        assert_eq!(
            result
                .iter()
                .find(|s| s.id == inbound.id)
                .unwrap()
                .direction,
            Direction::Receive
        );
    }

    #[test]
    fn distinct_orders_without_ids_stay_separate_and_native_revisions_merge() {
        let accepted = notification("叮咚买菜", "正在打包商品", "a", NOW - 30_000).unwrap();
        let moving = notification("叮咚买菜", "骑手正在配送", "a", NOW - 20_000).unwrap();
        let delivered = notification("叮咚买菜", "商品已送达", "b", NOW).unwrap();
        let result = reconcile(vec![accepted, moving, delivered]);
        assert_eq!(
            result.iter().map(|s| s.stage).collect::<Vec<_>>(),
            [Stage::Arrived, Stage::Moving]
        );
    }

    #[test]
    fn agent_only_accepts_attested_phrase_and_sends_no_source_content() {
        let candidate = notification(
            "顺丰速运",
            "收件联系顾客甲 13800002916，快件 SF123456789012 正在派送，将送至您处",
            "private-key",
            NOW,
        )
        .unwrap();
        let vague =
            notification("顺丰速运", "您的快件 SF123456789012 正在派送", "vague", NOW).unwrap();
        let sent = notification(
            "顺丰速运",
            "您寄出的快件 SF123456789012 正在派送，将送至您处",
            "sent",
            NOW,
        )
        .unwrap();
        assert!(candidate.agent_candidate());
        assert!(!vague.agent_candidate());
        assert!(!sent.agent_candidate());
        let prompt = agent_request(DEFAULT_AGENT_PROMPT, &[&candidate]);
        for secret in ["顾客甲", "13800002916", "SF123456789012", "private-key"] {
            assert!(!prompt.contains(secret));
        }
        for evidence in ["不存在", "您的快件", ""] {
            let raw = serde_json::json!({"decisions":[{"id":0,"direction":"receive","evidence":evidence}]});
            assert!(validate_agent_response(&raw.to_string(), &[&candidate]).is_none());
        }
        let raw = r#"{"decisions":[{"id":0,"direction":"receive","evidence":"送至您处"}]}"#;
        assert_eq!(
            validate_agent_response(raw, &[&candidate])
                .unwrap()
                .get(&candidate.agent_key()),
            Some(&true)
        );
        assert!(validate_agent_response(raw, &[&sent]).is_none());
        let duplicate = r#"{"decisions":[{"id":0,"direction":"receive","evidence":"送至您处"},{"id":0,"direction":"unknown","evidence":""}]}"#;
        assert!(validate_agent_response(duplicate, &[&candidate, &vague]).is_none());
    }
}
