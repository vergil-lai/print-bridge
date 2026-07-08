package main

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"

	"golang.org/x/crypto/argon2"
)

const (
	encryptedFormat = "printbridge-config-encrypted"
	payloadFormat   = "printbridge-config"
	version         = 1
	saltBytes       = 16
	nonceBytes      = 12
	keyBytes        = 32
	memoryKiB       = 19456
	iterations      = 2
	parallelism     = 1
	tagBytes        = 16
	expectedPayload = "OTX3vkYug76bv335qmWdp85pbgu85QfwarlnqhxGoV0U+4sRez0dlwWy+5eIe597KLRqdHg7XJVbjLds/mXROcLHLhTJrJJ+DWpB2Xc6BX2sKii+bziOsb8akhUwxqo="
)

type EncryptOptions struct {
	Salt  []byte
	Nonce []byte
}

type EncryptedConfigFile struct {
	Format  string         `json:"format"`
	Version int            `json:"version"`
	Crypto  CryptoMetadata `json:"crypto"`
	Payload string         `json:"payload"`
}

type CryptoMetadata struct {
	KDF         string `json:"kdf"`
	MemoryKiB   uint32 `json:"memory_kib"`
	Iterations  uint32 `json:"iterations"`
	Parallelism uint8  `json:"parallelism"`
	Cipher      string `json:"cipher"`
	TagBytes    int    `json:"tag_bytes"`
	Salt        string `json:"salt"`
	Nonce       string `json:"nonce"`
}

type transferPayload struct {
	Format  string `json:"format"`
	Version int    `json:"version"`
	Config  struct {
		Service struct {
			Port int `json:"port"`
		} `json:"service"`
	} `json:"config"`
}

func EncryptConfigFile(payload any, password string, options EncryptOptions) (*EncryptedConfigFile, error) {
	salt, err := fixedOrRandom(options.Salt, saltBytes)
	if err != nil {
		return nil, err
	}
	nonce, err := fixedOrRandom(options.Nonce, nonceBytes)
	if err != nil {
		return nil, err
	}

	key := argon2.IDKey([]byte(password), salt, iterations, memoryKiB, parallelism, keyBytes)
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	gcm, err := cipher.NewGCMWithTagSize(block, tagBytes)
	if err != nil {
		return nil, err
	}

	plaintext, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	encryptedPayload := gcm.Seal(nil, nonce, plaintext, nil)

	return &EncryptedConfigFile{
		Format:  encryptedFormat,
		Version: version,
		Crypto: CryptoMetadata{
			KDF:         "argon2id13",
			MemoryKiB:   memoryKiB,
			Iterations:  iterations,
			Parallelism: parallelism,
			Cipher:      "aes-256-gcm",
			TagBytes:    tagBytes,
			Salt:        base64.StdEncoding.EncodeToString(salt),
			Nonce:       base64.StdEncoding.EncodeToString(nonce),
		},
		Payload: base64.StdEncoding.EncodeToString(encryptedPayload),
	}, nil
}

func DecryptConfigFile(file EncryptedConfigFile, password string) (map[string]any, error) {
	if err := validateEnvelope(file); err != nil {
		return nil, err
	}

	salt, err := decodeFixed(file.Crypto.Salt, saltBytes)
	if err != nil {
		return nil, err
	}
	nonce, err := decodeFixed(file.Crypto.Nonce, nonceBytes)
	if err != nil {
		return nil, err
	}
	encryptedPayload, err := base64.StdEncoding.DecodeString(file.Payload)
	if err != nil {
		return nil, err
	}

	key := argon2.IDKey([]byte(password), salt, iterations, memoryKiB, parallelism, keyBytes)
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	gcm, err := cipher.NewGCMWithTagSize(block, tagBytes)
	if err != nil {
		return nil, err
	}
	plaintext, err := gcm.Open(nil, nonce, encryptedPayload, nil)
	if err != nil {
		return nil, errors.New("invalid password or encrypted config payload")
	}

	var payload map[string]any
	if err := json.Unmarshal(plaintext, &payload); err != nil {
		return nil, err
	}
	if payload["format"] != payloadFormat || payload["version"] != float64(version) {
		return nil, errors.New("invalid decrypted config payload")
	}
	return payload, nil
}

func fixedOrRandom(value []byte, length int) ([]byte, error) {
	if value != nil {
		if len(value) != length {
			return nil, errors.New("invalid salt or nonce length")
		}
		return value, nil
	}

	out := make([]byte, length)
	if _, err := io.ReadFull(rand.Reader, out); err != nil {
		return nil, err
	}
	return out, nil
}

func validateEnvelope(file EncryptedConfigFile) error {
	if file.Format != encryptedFormat ||
		file.Version != version ||
		file.Crypto.KDF != "argon2id13" ||
		file.Crypto.MemoryKiB != memoryKiB ||
		file.Crypto.Iterations != iterations ||
		file.Crypto.Parallelism != parallelism ||
		file.Crypto.Cipher != "aes-256-gcm" ||
		file.Crypto.TagBytes != tagBytes {
		return errors.New("invalid encrypted config file")
	}
	return nil
}

func decodeFixed(value string, length int) ([]byte, error) {
	bytes, err := base64.StdEncoding.DecodeString(value)
	if err != nil {
		return nil, err
	}
	if len(bytes) != length {
		return nil, errors.New("invalid fixed-length field")
	}
	return bytes, nil
}

func verify() error {
	payload := transferPayload{
		Format:  payloadFormat,
		Version: version,
	}
	payload.Config.Service.Port = 17890

	salt, _ := hex.DecodeString("000102030405060708090a0b0c0d0e0f")
	nonce, _ := hex.DecodeString("101112131415161718191a1b")
	file, err := EncryptConfigFile(payload, "test-password", EncryptOptions{Salt: salt, Nonce: nonce})
	if err != nil {
		return err
	}
	if file.Payload != expectedPayload {
		return errors.New("encrypted payload does not match expected vector")
	}

	decrypted, err := DecryptConfigFile(*file, "test-password")
	if err != nil {
		return err
	}
	config := decrypted["config"].(map[string]any)
	service := config["service"].(map[string]any)
	if service["port"] != float64(17890) {
		return errors.New("decrypted payload did not round-trip")
	}

	encoded, err := json.MarshalIndent(file, "", "  ")
	if err != nil {
		return err
	}
	fmt.Println(string(encoded))
	return nil
}

func main() {
	if len(os.Args) > 1 && os.Args[1] == "verify" {
		if err := verify(); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
	}
}
