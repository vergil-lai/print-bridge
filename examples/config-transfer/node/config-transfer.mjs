import crypto from 'node:crypto';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const sodium = require('libsodium-wrappers-sumo');

const ENCRYPTED_FORMAT = 'printbridge-config-encrypted';
const PAYLOAD_FORMAT = 'printbridge-config';
const VERSION = 1;
const SALT_BYTES = 16;
const NONCE_BYTES = 12;
const KEY_BYTES = 32;
const MEMORY_KIB = 19456;
const ITERATIONS = 2;
const PARALLELISM = 1;
const TAG_BYTES = 16;
const EXPECTED_PAYLOAD =
  'OTX3vkYug76bv335qmWdp85pbgu85QfwarlnqhxGoV0U+4sRez0dlwWy+5eIe597KLRqdHg7XJVbjLds/mXROcLHLhTJrJJ+DWpB2Xc6BX2sKii+bziOsb8akhUwxqo=';

await sodium.ready;

export function encryptConfigFile(payload, password, options = {}) {
  const salt = fixedOrRandom(options.salt, SALT_BYTES);
  const nonce = fixedOrRandom(options.nonce, NONCE_BYTES);
  const key = deriveKey(password, salt);
  const cipher = crypto.createCipheriv('aes-256-gcm', key, nonce, {
    authTagLength: TAG_BYTES,
  });
  const ciphertext = Buffer.concat([
    cipher.update(JSON.stringify(payload), 'utf8'),
    cipher.final(),
  ]);
  const tag = cipher.getAuthTag();

  return {
    format: ENCRYPTED_FORMAT,
    version: VERSION,
    crypto: {
      kdf: 'argon2id13',
      memory_kib: MEMORY_KIB,
      iterations: ITERATIONS,
      parallelism: PARALLELISM,
      cipher: 'aes-256-gcm',
      tag_bytes: TAG_BYTES,
      salt: Buffer.from(salt).toString('base64'),
      nonce: Buffer.from(nonce).toString('base64'),
    },
    payload: Buffer.concat([ciphertext, tag]).toString('base64'),
  };
}

export function decryptConfigFile(file, password) {
  validateEnvelope(file);

  const salt = decodeFixed(file.crypto.salt, SALT_BYTES);
  const nonce = decodeFixed(file.crypto.nonce, NONCE_BYTES);
  const encryptedPayload = Buffer.from(file.payload, 'base64');
  if (encryptedPayload.length < TAG_BYTES) {
    throw new Error('Invalid encrypted config payload.');
  }

  const ciphertext = encryptedPayload.subarray(0, -TAG_BYTES);
  const tag = encryptedPayload.subarray(-TAG_BYTES);
  const decipher = crypto.createDecipheriv('aes-256-gcm', deriveKey(password, salt), nonce, {
    authTagLength: TAG_BYTES,
  });
  decipher.setAuthTag(tag);

  let plaintext;
  try {
    plaintext = Buffer.concat([decipher.update(ciphertext), decipher.final()]).toString('utf8');
  } catch {
    throw new Error('Invalid password or encrypted config payload.');
  }

  const payload = JSON.parse(plaintext);
  if (payload.format !== PAYLOAD_FORMAT || payload.version !== VERSION) {
    throw new Error('Invalid decrypted config payload.');
  }
  return payload;
}

function deriveKey(password, salt) {
  return Buffer.from(
    sodium.crypto_pwhash(
      KEY_BYTES,
      password,
      salt,
      ITERATIONS,
      MEMORY_KIB * 1024,
      sodium.crypto_pwhash_ALG_ARGON2ID13,
    ),
  );
}

function fixedOrRandom(value, length) {
  if (value) {
    const bytes = Buffer.from(value);
    if (bytes.length !== length) {
      throw new Error('Invalid salt or nonce length.');
    }
    return bytes;
  }
  return crypto.randomBytes(length);
}

function decodeFixed(value, length) {
  const bytes = Buffer.from(value, 'base64');
  if (bytes.length !== length) {
    throw new Error('Invalid fixed-length field.');
  }
  return bytes;
}

function validateEnvelope(file) {
  if (
    file?.format !== ENCRYPTED_FORMAT ||
    file.version !== VERSION ||
    file.crypto?.kdf !== 'argon2id13' ||
    file.crypto.memory_kib !== MEMORY_KIB ||
    file.crypto.iterations !== ITERATIONS ||
    file.crypto.parallelism !== PARALLELISM ||
    file.crypto.cipher !== 'aes-256-gcm' ||
    file.crypto.tag_bytes !== TAG_BYTES
  ) {
    throw new Error('Invalid encrypted config file.');
  }
}

function verify() {
  const payload = {
    format: PAYLOAD_FORMAT,
    version: VERSION,
    config: {
      service: {
        port: 17890,
      },
    },
  };
  const file = encryptConfigFile(payload, 'test-password', {
    salt: Buffer.from('000102030405060708090a0b0c0d0e0f', 'hex'),
    nonce: Buffer.from('101112131415161718191a1b', 'hex'),
  });

  if (file.payload !== EXPECTED_PAYLOAD) {
    throw new Error('Encrypted payload does not match expected vector.');
  }

  const decrypted = decryptConfigFile(file, 'test-password');
  if (decrypted.config.service.port !== 17890) {
    throw new Error('Decrypted payload did not round-trip.');
  }

  console.log(JSON.stringify(file, null, 2));
}

if (process.argv[2] === 'verify') {
  verify();
}
