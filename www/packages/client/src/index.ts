// @mediaservo/client — cockpit/consumer-side SDK (TS surface)
// P0/D270-R1: transport (the former admin sfu-client, logic unchanged) + auth pure surface + server wire types.
// From P1 on, transport swaps its core to mediasoup-client (resilience layer kept); see docs/plans/client-dual-form/.

export * from './transport/sfu-client';
export * from './auth';
export * from './types';
