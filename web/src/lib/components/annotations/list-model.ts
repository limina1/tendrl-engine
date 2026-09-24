// Compatibility shim: the mobile list became the ledger's rows
// (ledger-model.ts — docs/annotations-ledger-contract.org). The helpers
// other modules import under the old path live there now.
export { relTime, authorFallback } from './ledger-model';
