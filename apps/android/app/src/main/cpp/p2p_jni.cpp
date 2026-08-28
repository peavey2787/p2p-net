#include <jni.h>

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

extern "C" {
char *p2p_android_default_config_json();
char *p2p_android_validate_config(const uint8_t *config_json, size_t config_len);
char *p2p_android_start(const uint8_t *config_json, size_t config_len, const uint8_t *data_dir,
                        size_t data_dir_len);
char *p2p_android_stop();
uint64_t p2p_android_revision();
char *p2p_android_snapshot_json();
char *p2p_android_peers_json();
char *p2p_android_metrics_json();
char *p2p_android_bridge_stats_json();
char *p2p_android_connect(const uint8_t *addr, size_t addr_len);
char *p2p_android_disconnect(const uint8_t *peer_id, size_t peer_id_len);
char *p2p_android_broadcast(const uint8_t *topic, size_t topic_len, const uint8_t *payload,
                            size_t payload_len);
char *p2p_android_send(const uint8_t *peer_id, size_t peer_id_len, const uint8_t *topic,
                       size_t topic_len, const uint8_t *payload, size_t payload_len);
char *p2p_android_subscribe(const uint8_t *topic, size_t topic_len);
uint32_t p2p_android_pending_message_count();
char *p2p_android_drain_messages_json(uint32_t max_messages);
void p2p_android_string_free(char *value);
}

namespace {
constexpr jsize kMaxPayloadBytes = 1024 * 1024;
constexpr size_t kMaxConfigBytes = 256 * 1024;
constexpr size_t kMaxDataDirBytes = 4 * 1024;
constexpr size_t kMaxMultiaddrBytes = 4 * 1024;
constexpr size_t kMaxPeerIdBytes = 256;
constexpr size_t kMaxTopicBytes = 128;

bool jstring_to_utf8(JNIEnv *env, jstring value, size_t max_bytes, std::string *out) {
    if (value == nullptr) {
        return false;
    }
    const jsize length = env->GetStringLength(value);
    if (length < 0 || static_cast<size_t>(length) > max_bytes) {
        return false;
    }
    const jchar *chars = env->GetStringChars(value, nullptr);
    if (chars == nullptr) {
        return false;
    }

    out->clear();
    out->reserve(static_cast<size_t>(length));
    for (jsize i = 0; i < length; ++i) {
        uint32_t codepoint = chars[i];
        if (codepoint >= 0xD800 && codepoint <= 0xDBFF) {
            if (i + 1 >= length) {
                env->ReleaseStringChars(value, chars);
                out->clear();
                return false;
            }
            const uint32_t low = chars[i + 1];
            if (low < 0xDC00 || low > 0xDFFF) {
                env->ReleaseStringChars(value, chars);
                out->clear();
                return false;
            }
            codepoint = 0x10000 + ((codepoint - 0xD800) << 10) + (low - 0xDC00);
            ++i;
        } else if (codepoint >= 0xDC00 && codepoint <= 0xDFFF) {
            env->ReleaseStringChars(value, chars);
            out->clear();
            return false;
        }
        if (codepoint <= 0x7F) {
            out->push_back(static_cast<char>(codepoint));
        } else if (codepoint <= 0x7FF) {
            out->push_back(static_cast<char>(0xC0 | (codepoint >> 6)));
            out->push_back(static_cast<char>(0x80 | (codepoint & 0x3F)));
        } else if (codepoint <= 0xFFFF) {
            out->push_back(static_cast<char>(0xE0 | (codepoint >> 12)));
            out->push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3F)));
            out->push_back(static_cast<char>(0x80 | (codepoint & 0x3F)));
        } else {
            out->push_back(static_cast<char>(0xF0 | (codepoint >> 18)));
            out->push_back(static_cast<char>(0x80 | ((codepoint >> 12) & 0x3F)));
            out->push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3F)));
            out->push_back(static_cast<char>(0x80 | (codepoint & 0x3F)));
        }
        if (out->size() > max_bytes) {
            env->ReleaseStringChars(value, chars);
            out->clear();
            return false;
        }
    }
    env->ReleaseStringChars(value, chars);
    return true;
}

bool utf8_to_utf16(const char *value, std::vector<jchar> *out) {
    if (value == nullptr) {
        return false;
    }
    const auto *bytes = reinterpret_cast<const uint8_t *>(value);
    size_t i = 0;
    while (bytes[i] != 0) {
        uint32_t cp = 0;
        size_t extra = 0;
        if (bytes[i] <= 0x7F) {
            cp = bytes[i];
        } else if ((bytes[i] & 0xE0) == 0xC0) {
            cp = bytes[i] & 0x1F;
            extra = 1;
        } else if ((bytes[i] & 0xF0) == 0xE0) {
            cp = bytes[i] & 0x0F;
            extra = 2;
        } else if ((bytes[i] & 0xF8) == 0xF0) {
            cp = bytes[i] & 0x07;
            extra = 3;
        } else {
            return false;
        }
        for (size_t j = 1; j <= extra; ++j) {
            const uint8_t next = bytes[i + j];
            if (next == 0 || (next & 0xC0) != 0x80) {
                return false;
            }
            cp = (cp << 6) | (next & 0x3F);
        }
        i += extra + 1;
        if (cp <= 0xFFFF) {
            if (cp >= 0xD800 && cp <= 0xDFFF) {
                return false;
            }
            out->push_back(static_cast<jchar>(cp));
        } else if (cp <= 0x10FFFF) {
            cp -= 0x10000;
            out->push_back(static_cast<jchar>(0xD800 | (cp >> 10)));
            out->push_back(static_cast<jchar>(0xDC00 | (cp & 0x3FF)));
        } else {
            return false;
        }
    }
    return true;
}

jstring rust_string_to_jstring(JNIEnv *env, char *value) {
    if (value == nullptr) {
        return env->NewStringUTF("{\"ok\":false,\"error\":\"native allocation failed\"}");
    }
    std::vector<jchar> utf16;
    const bool valid = utf8_to_utf16(value, &utf16);
    p2p_android_string_free(value);
    if (!valid) {
        return env->NewStringUTF(
            "{\"ok\":false,\"error\":\"native bridge returned invalid UTF-8\"}");
    }
    const jchar empty = 0;
    const jchar *data = utf16.empty() ? &empty : utf16.data();
    return env->NewString(data, static_cast<jsize>(utf16.size()));
}

jstring input_error(JNIEnv *env, const char *message) {
    const std::string json = std::string("{\"ok\":false,\"error\":\"") + message + "\"}";
    return env->NewStringUTF(json.c_str());
}

std::vector<uint8_t> jbytes_to_vector(JNIEnv *env, jbyteArray value, bool *ok) {
    *ok = false;
    if (value == nullptr) {
        return {};
    }
    const jsize length = env->GetArrayLength(value);
    if (length < 0 || length > kMaxPayloadBytes) {
        return {};
    }
    std::vector<uint8_t> bytes(static_cast<size_t>(length));
    if (length > 0) {
        env->GetByteArrayRegion(value, 0, length, reinterpret_cast<jbyte *>(bytes.data()));
        if (env->ExceptionCheck()) {
            return {};
        }
    }
    *ok = true;
    return bytes;
}

jstring payload_error(JNIEnv *env) {
    return input_error(env, "payload exceeds 1 MiB or could not be read");
}
}  // namespace

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_defaultConfig(JNIEnv *env, jobject) {
    return rust_string_to_jstring(env, p2p_android_default_config_json());
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_validateConfig(JNIEnv *env, jobject, jstring config) {
    std::string config_utf8;
    if (!jstring_to_utf8(env, config, kMaxConfigBytes, &config_utf8)) {
        return input_error(env, "config is null, invalid, or exceeds 256 KiB");
    }
    return rust_string_to_jstring(
        env,
        p2p_android_validate_config(reinterpret_cast<const uint8_t *>(config_utf8.data()),
                                    config_utf8.size()));
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_start(JNIEnv *env, jobject, jstring config,
                                                   jstring data_dir) {
    std::string config_utf8;
    std::string data_dir_utf8;
    if (!jstring_to_utf8(env, config, kMaxConfigBytes, &config_utf8)) {
        return input_error(env, "config is null, invalid, or exceeds 256 KiB");
    }
    if (!jstring_to_utf8(env, data_dir, kMaxDataDirBytes, &data_dir_utf8)) {
        return input_error(env, "data directory is null, invalid, or exceeds 4 KiB");
    }
    return rust_string_to_jstring(
        env,
        p2p_android_start(reinterpret_cast<const uint8_t *>(config_utf8.data()),
                          config_utf8.size(),
                          reinterpret_cast<const uint8_t *>(data_dir_utf8.data()),
                          data_dir_utf8.size()));
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_stop(JNIEnv *env, jobject) {
    return rust_string_to_jstring(env, p2p_android_stop());
}

extern "C" JNIEXPORT jlong JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_revision(JNIEnv *, jobject) {
    return static_cast<jlong>(p2p_android_revision());
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_snapshot(JNIEnv *env, jobject) {
    return rust_string_to_jstring(env, p2p_android_snapshot_json());
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_peers(JNIEnv *env, jobject) {
    return rust_string_to_jstring(env, p2p_android_peers_json());
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_metrics(JNIEnv *env, jobject) {
    return rust_string_to_jstring(env, p2p_android_metrics_json());
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_bridgeStats(JNIEnv *env, jobject) {
    return rust_string_to_jstring(env, p2p_android_bridge_stats_json());
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_connect(JNIEnv *env, jobject, jstring addr) {
    std::string addr_utf8;
    if (!jstring_to_utf8(env, addr, kMaxMultiaddrBytes, &addr_utf8)) {
        return input_error(env, "multiaddr is null, invalid, or exceeds 4 KiB");
    }
    return rust_string_to_jstring(
        env, p2p_android_connect(reinterpret_cast<const uint8_t *>(addr_utf8.data()),
                                 addr_utf8.size()));
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_disconnect(JNIEnv *env, jobject, jstring peer_id) {
    std::string peer_utf8;
    if (!jstring_to_utf8(env, peer_id, kMaxPeerIdBytes, &peer_utf8)) {
        return input_error(env, "peer id is null, invalid, or exceeds 256 bytes");
    }
    return rust_string_to_jstring(
        env, p2p_android_disconnect(reinterpret_cast<const uint8_t *>(peer_utf8.data()),
                                    peer_utf8.size()));
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_broadcast(JNIEnv *env, jobject, jstring topic,
                                                       jbyteArray payload) {
    std::string topic_utf8;
    if (!jstring_to_utf8(env, topic, kMaxTopicBytes, &topic_utf8)) {
        return input_error(env, "topic is null, invalid, or exceeds 128 bytes");
    }
    bool ok = false;
    auto bytes = jbytes_to_vector(env, payload, &ok);
    if (!ok) {
        return payload_error(env);
    }
    return rust_string_to_jstring(
        env, p2p_android_broadcast(reinterpret_cast<const uint8_t *>(topic_utf8.data()),
                                   topic_utf8.size(), bytes.data(), bytes.size()));
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_send(JNIEnv *env, jobject, jstring peer_id,
                                                  jstring topic, jbyteArray payload) {
    std::string peer_utf8;
    std::string topic_utf8;
    if (!jstring_to_utf8(env, peer_id, kMaxPeerIdBytes, &peer_utf8)) {
        return input_error(env, "peer id is null, invalid, or exceeds 256 bytes");
    }
    if (!jstring_to_utf8(env, topic, kMaxTopicBytes, &topic_utf8)) {
        return input_error(env, "topic is null, invalid, or exceeds 128 bytes");
    }
    bool ok = false;
    auto bytes = jbytes_to_vector(env, payload, &ok);
    if (!ok) {
        return payload_error(env);
    }
    return rust_string_to_jstring(
        env, p2p_android_send(reinterpret_cast<const uint8_t *>(peer_utf8.data()),
                              peer_utf8.size(),
                              reinterpret_cast<const uint8_t *>(topic_utf8.data()),
                              topic_utf8.size(), bytes.data(), bytes.size()));
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_subscribe(JNIEnv *env, jobject, jstring topic) {
    std::string topic_utf8;
    if (!jstring_to_utf8(env, topic, kMaxTopicBytes, &topic_utf8)) {
        return input_error(env, "topic is null, invalid, or exceeds 128 bytes");
    }
    return rust_string_to_jstring(
        env, p2p_android_subscribe(reinterpret_cast<const uint8_t *>(topic_utf8.data()),
                                   topic_utf8.size()));
}

extern "C" JNIEXPORT jint JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_pendingMessageCount(JNIEnv *, jobject) {
    return static_cast<jint>(p2p_android_pending_message_count());
}

extern "C" JNIEXPORT jstring JNICALL
Java_io_github_peavey2787_p2pnet_NativeNode_drainMessages(JNIEnv *env, jobject, jint max_messages) {
    const uint32_t bounded = max_messages <= 0 ? 0u : static_cast<uint32_t>(max_messages);
    return rust_string_to_jstring(env, p2p_android_drain_messages_json(bounded));
}
