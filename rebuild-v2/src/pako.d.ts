declare module 'pako' {
  interface PakoRuntime {
    deflate(input: string): Uint8Array;
    inflate(input: Uint8Array): Uint8Array;
  }

  const pako: PakoRuntime;
  export default pako;
}
