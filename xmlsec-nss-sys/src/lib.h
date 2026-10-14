#define XMLSEC_CRYPTO_NSS
#define XMLSEC_NO_CRYPTO_DYNAMIC_LOADING
#define XMLSEC_NO_XSLT

#include <cert.h>
#include <pk11pub.h>

#include <xmlsec/templates.h>
#include <xmlsec/xmldsig.h>
#include <xmlsec/xmlsec.h>
#include <xmlsec/xmltree.h>

#include <xmlsec/nss/app.h>
#include <xmlsec/nss/crypto.h>

#include <xmlsec/nss/pkikeys.h>

#include <keyhi.h>
