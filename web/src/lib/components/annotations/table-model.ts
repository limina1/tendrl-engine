// Compatibility shim: the table became the ledger (ledger-model.ts —
// docs/annotations-ledger-contract.org). The id helpers other modules
// import under the old path live there now; import from ledger-model in
// new code.
export { annotationsBufferId, rootFromAnnotationsId, rootFromReaderId, flattenToc } from './ledger-model';
export type { Section } from './ledger-model';
