import json, time, secrets, struct, hashlib, urllib.request
from nacl.signing import SigningKey
url='http://127.0.0.1:8080'
nsk='b6dd606a5390798bcc4d1f81508064930bc76fdb0de139d6fd187f938d846092'
npk='0dcc7432a97fbd69368cc7a865d0e2c42a4a9312b4f06e532e7de33c5ac349b0'
sk=SigningKey(bytes.fromhex(nsk)); ts=int(time.time()); nonce=secrets.token_hex(16)
sig=sk.sign(f'baals-auth-token:{ts}:{nonce}'.encode()).signature.hex()
req=urllib.request.Request(f'{url}/api/v1/auth/token',data=json.dumps({'timestamp':ts,'nonce':nonce,'public_key':npk,'signature':sig,'ttl_seconds':900}).encode(),headers={'Content-Type':'application/json'},method='POST')
token=json.loads(urllib.request.urlopen(req).read().decode())['token']
ask='aaaa000000000000000000000000000000000000000000000000000000000001'
apk=SigningKey(bytes.fromhex(ask)).verify_key.encode().hex()
bpk=SigningKey(bytes.fromhex('bbbb000000000000000000000000000000000000000000000000000000000002')).verify_key.encode().hex()
tx_ts=int(time.time())
buf=bytes.fromhex(apk)+struct.pack('<Q',5)+struct.pack('<Q',tx_ts)+struct.pack('<Q',100000)+struct.pack('<Q',1)+struct.pack('<B',0)+struct.pack('<Q',1)+struct.pack('<I',0)+struct.pack('<Q',32)+bytes.fromhex(bpk)+struct.pack('<I',0)+struct.pack('<Q',75)
h=hashlib.sha256(buf).digest()
s=SigningKey(bytes.fromhex(ask)).sign(h).signature
tx={'hash':list(h),'sender':list(bytes.fromhex(apk)),'nonce':5,'timestamp':tx_ts,'recipient':{'Wallet':list(bytes.fromhex(bpk))},'payload':{'Transfer':{'amount':75}},'signature':list(s),'gas_limit':100000,'gas_price':1,'priority':0,'metadata':None,'chain_id':1}
req=urllib.request.Request(f'{url}/api/v1/transactions',data=json.dumps(tx).encode(),headers={'Content-Type':'application/json','Authorization':f'Bearer {token}'},method='POST')
print(urllib.request.urlopen(req).read().decode())
