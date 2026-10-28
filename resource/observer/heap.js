// Every JS resource shares FXServer's one Node runtime, so its memory is the JS side's footprint.
const HEAP_PERIOD_MS = 10000;

setInterval(() => {
  const memory = process.memoryUsage();
  PrintStructuredTrace(
    JSON.stringify({
      type: 'observer',
      kind: 'heap',
      heapUsed: memory.heapUsed,
      heapTotal: memory.heapTotal,
      rss: memory.rss,
      external: memory.external,
    }),
  );
}, HEAP_PERIOD_MS);
