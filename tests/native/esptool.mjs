// ESPLoader supplies framing, checksums, cumulative ACKs and status parsing.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {readFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';
import {deflateSync} from 'node:zlib';

const [bundle, executable, fixture] = process.argv.slice(2);
const {ESPLoader} = await import(pathToFileURL(bundle));
const child = spawn(executable, {stdio:['pipe','pipe','inherit']});
const chunks = child.stdout[Symbol.asyncIterator]();
let pending = Buffer.alloc(0);
async function receive() {
  while (pending.length < 4 || pending.length < 4 + pending.readUInt32LE()) {
    const {done,value} = await chunks.next();
    if (done) throw Error('peer closed before response');
    pending = Buffer.concat([pending,value]);
  }
  const end = 4 + pending.readUInt32LE(), frame = pending.subarray(4,end);
  pending = pending.subarray(end);
  return frame;
}
const transport = {
  getInfo: () => 'native NOR simulator',
  flushInput() {},
  read: receive,
  async write(frame) {
    const header = Buffer.alloc(4);
    header.writeUInt32LE(frame.length);
    child.stdin.write(Buffer.concat([header,frame]));
  },
};
try {
  const loader = new ESPLoader({transport, baudrate:115200});
  loader.IS_STUB = true;
  loader.chip = {CHIP_NAME:'ESP32-P4',getEraseSize:(_offset,size)=>size};
  loader.FLASH_WRITE_SIZE = 2048;
  const input = readFileSync(fixture);
  const expected = createHash('md5').update(input).digest('hex');
  await assert.rejects(loader.readFlash(4096, input.length), /Failed to read flash failed with status/);
  for (const compressed of [false,true]) {
    const data = compressed ? deflateSync(input) : input;
    const blocks = compressed
      ? await loader.flashDeflBegin(input.length,data.length,4096)
      : await loader.flashBegin(input.length,4096);
    for (let seq = 0; seq < blocks; seq++) {
      const block = data.subarray(seq*2048,(seq+1)*2048);
      if (compressed) await loader.flashDeflBlock(block,seq,3000);
      else {
        const padded = Buffer.alloc(2048,0xff);
        block.copy(padded);
        await loader.flashBlock(padded,seq,3000);
      }
    }
    if (compressed) await loader.flashDeflFinish(false);
    else await loader.flashFinish(false);
    const read = await loader.readFlash(4096,input.length);
    assert.deepEqual(Buffer.from(read),input);
    assert.equal((await receive()).toString('hex'),expected);
    assert.equal(await loader.flashMd5sum(4096,input.length),expected);
  }
  await assert.rejects(loader.flashMd5sum(262140,8));
  await loader.eraseFlash();
  const erased = Buffer.alloc(4096,0xff);
  assert.equal(await loader.flashMd5sum(4096,4096),createHash('md5').update(erased).digest('hex'));
  console.log('esptool-js raw/deflate, multi-packet read, stream MD5 and SPI_FLASH_MD5 passed');
} finally {
  child.stdin.end();
  child.kill();
}
