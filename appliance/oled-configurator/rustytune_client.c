#define _POSIX_C_SOURCE 200809L
#include "rustytune_client.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static int request(const char *json, char *reply, size_t reply_size, char *message, size_t size) {
    int fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (fd < 0) { snprintf(message, size, "Socket failed: %s", strerror(errno)); return -1; }
    struct sockaddr_un address = {0}; address.sun_family = AF_UNIX;
    snprintf(address.sun_path, sizeof(address.sun_path), "%s", RUSTYTUNE_ADMIN_SOCKET);
    if (connect(fd, (struct sockaddr *)&address, sizeof(address)) != 0) {
        bool artifacts_present = access("/usr/local/bin/rustytune", X_OK) == 0 ||
                                 access("/etc/rustytune/rustytune.toml", F_OK) == 0 ||
                                 access("/etc/systemd/system/rustytune.service", F_OK) == 0;
        snprintf(message, size, "%s", errno == ENOENT && !artifacts_present ?
                 "RustyTune not installed" : "RustyTune service unavailable");
        close(fd); return -1;
    }
    char line[768]; int length = snprintf(line, sizeof(line), "%s\n", json);
    if (length < 0 || (size_t)length >= sizeof(line) || write(fd, line, (size_t)length) != length) { snprintf(message,size,"Admin request failed"); close(fd); return -1; }
    size_t used = 0;
    while (used + 1 < reply_size) { ssize_t got=read(fd,reply+used,reply_size-used-1); if(got<=0)break; used+=(size_t)got; if(memchr(reply,'\n',used))break; }
    close(fd); reply[used]='\0'; char *newline=strchr(reply,'\n'); if(newline)*newline='\0';
    if (strstr(reply,"\"error\"")) { const char *p=strstr(reply,"\"error\":"); snprintf(message,size,"%.100s",p?p+8:"RustyTune error"); return -1; }
    snprintf(message,size,"OK"); return 0;
}

static bool json_bool(const char *json,const char *key) { char needle[64]; snprintf(needle,sizeof(needle),"\"%s\":true",key); return strstr(json,needle)!=NULL; }
static unsigned long long json_uint(const char *json,const char *key) { char needle[64]; snprintf(needle,sizeof(needle),"\"%s\":",key); const char *p=strstr(json,needle); return p?strtoull(p+strlen(needle),NULL,10):0; }
static void json_string(const char *json,const char *key,char *out,size_t size) { char needle[64]; snprintf(needle,sizeof(needle),"\"%s\":\"",key); const char *p=strstr(json,needle); if(!p){out[0]='\0';return;} p+=strlen(needle); const char *end=strchr(p,'\"'); size_t n=end?(size_t)(end-p):0; if(n>=size)n=size-1; memcpy(out,p,n);out[n]='\0'; }

int rustytune_get_status(rustytune_status_t *s,char *message,size_t size){memset(s,0,sizeof(*s));char reply[1024];if(request("{\"command\":\"status\"}",reply,sizeof(reply),message,size))return -1;s->installed=true;s->connected=json_bool(reply,"connected");s->logging=strstr(reply,"\"log\":{")!=NULL;json_string(reply,"port",s->port,sizeof(s->port));json_string(reply,"mode",s->mode,sizeof(s->mode));s->baud=(unsigned)json_uint(reply,"baud");s->frames=json_uint(reply,"frames");s->timeouts=json_uint(reply,"timeouts");s->crc_errors=json_uint(reply,"crcErrors");s->log_rows=json_uint(reply,"rows");json_string(reply,"ecuSignature",s->ecu_signature,sizeof(s->ecu_signature));json_string(reply,"lastError",s->error,sizeof(s->error));return 0;}
int rustytune_get_config(rustytune_config_t *c,char *message,size_t size){memset(c,0,sizeof(*c));char reply[2048];if(request("{\"command\":\"config\"}",reply,sizeof(reply),message,size))return -1;json_string(reply,"device",c->device,sizeof(c->device));json_string(reply,"mode",c->mode,sizeof(c->mode));c->baud=(unsigned)json_uint(reply,"baud");c->auto_log=json_bool(reply,"auto");c->retention_bytes=json_uint(reply,"retention_bytes");return 0;}
int rustytune_open_pairing(char code[7],unsigned *expires,char *message,size_t size){char reply[512];if(request("{\"command\":\"pair\"}",reply,sizeof(reply),message,size))return -1;json_string(reply,"code",code,7);*expires=(unsigned)json_uint(reply,"expiresIn");return 0;}
int rustytune_reconnect(char *message,size_t size){char reply[256];return request("{\"command\":\"reconnect\"}",reply,sizeof(reply),message,size);}
int rustytune_restart(char *message,size_t size){char reply[256];return request("{\"command\":\"restart\"}",reply,sizeof(reply),message,size);}
int rustytune_update_connection(const rustytune_config_t *c,char *message,size_t size){char json[512],reply[512];snprintf(json,sizeof(json),"{\"command\":\"configure\",\"device\":\"%.90s\",\"mode\":\"%.12s\",\"baud\":%u,\"autoLog\":%s}",c->device,c->mode,c->baud,c->auto_log?"true":"false");return request(json,reply,sizeof(reply),message,size);}
