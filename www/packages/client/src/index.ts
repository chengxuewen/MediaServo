// @mediaservo/client — 座舱端/消费端 SDK（TS 面）
// P0/D270-R1：transport（原 admin sfu-client，零逻辑改动）+ auth 纯面 + server wire 类型。
// P1 起 transport 内包换心 mediasoup-client（韧性层保留），见 docs/plans/client-dual-form/。

export * from './transport/sfu-client';
export * from './auth';
export * from './types';
