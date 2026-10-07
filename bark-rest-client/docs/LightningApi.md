# \LightningApi

All URIs are relative to *http://localhost*

Method | HTTP request | Description
------------- | ------------- | -------------
[**abandon_hold_receive**](LightningApi.md#abandon_hold_receive) | **POST** /api/v1/lightning/receives/{payment_hash}/abandon | Abandon a hold receive
[**cancel_receive**](LightningApi.md#cancel_receive) | **DELETE** /api/v1/lightning/receives/{identifier} | Cancel a pending receive
[**generate_hold_invoice**](LightningApi.md#generate_hold_invoice) | **POST** /api/v1/lightning/receives/hold-invoice | Create a BOLT11 hold invoice
[**generate_invoice**](LightningApi.md#generate_invoice) | **POST** /api/v1/lightning/receives/invoice | Create a BOLT11 invoice
[**generate_invoice_for_address**](LightningApi.md#generate_invoice_for_address) | **POST** /api/v1/lightning/receives/invoice/for-address | Create a BOLT11 invoice for an Ark address
[**get_receive_status**](LightningApi.md#get_receive_status) | **GET** /api/v1/lightning/receives/{identifier} | Get receive status
[**get_send_status**](LightningApi.md#get_send_status) | **GET** /api/v1/lightning/sends/{identifier} | Get send status
[**list_receive_statuses**](LightningApi.md#list_receive_statuses) | **GET** /api/v1/lightning/receives | List all pending receive statuses
[**pay**](LightningApi.md#pay) | **POST** /api/v1/lightning/pay | Send a Lightning payment
[**settle_hold_receive**](LightningApi.md#settle_hold_receive) | **POST** /api/v1/lightning/receives/{payment_hash}/settle | Settle a hold receive



## abandon_hold_receive

> serde_json::Value abandon_hold_receive(payment_hash)
Abandon a hold receive

Gives up on a hold receive whose preimage has not been supplied. Before the payment arrived this cancels the receive. Once the HTLC-recv VTXOs are granted they are dropped unclaimed, and the Ark server fails the inbound payment back to the sender when the invoice expires.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**payment_hash** | **String** | Payment hash of the hold receive | [required] |

### Return type

[**serde_json::Value**](serde_json::Value.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## cancel_receive

> cancel_receive(identifier)
Cancel a pending receive

Cancels a pending Lightning receive identified by its payment hash or invoice string. The server will refuse cancellation if HTLC-recv VTXOs have already been granted. Bark also prevents cancellation when the preimage has been revealed.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**identifier** | **String** | Payment hash or invoice string | [required] |

### Return type

 (empty response body)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## generate_hold_invoice

> models::InvoiceInfo generate_hold_invoice(lightning_hold_invoice_request)
Create a BOLT11 hold invoice

Generates a BOLT11 invoice for a payment hash whose preimage the caller holds, creating a pending hold receive. Once the payment arrives and the Ark server has granted the HTLC-recv VTXOs, the receive's state becomes `awaiting_preimage` and it waits for the caller to settle it with the preimage or abandon it. A payment arriving after `expiry_secs` is not claimed. Fails if the wallet already has a receive for the payment hash.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**lightning_hold_invoice_request** | [**LightningHoldInvoiceRequest**](LightningHoldInvoiceRequest.md) |  | [required] |

### Return type

[**models::InvoiceInfo**](InvoiceInfo.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## generate_invoice

> models::InvoiceInfo generate_invoice(lightning_invoice_request)
Create a BOLT11 invoice

Generates a new BOLT11 invoice for the specified amount via the Ark server, creating a pending Lightning receive.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**lightning_invoice_request** | [**LightningInvoiceRequest**](LightningInvoiceRequest.md) |  | [required] |

### Return type

[**models::InvoiceInfo**](InvoiceInfo.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## generate_invoice_for_address

> models::InvoiceInfo generate_invoice_for_address(lightning_invoice_for_address_request)
Create a BOLT11 invoice for an Ark address

Generates a new BOLT11 invoice. When paid, the wallet claims the Lightning receive and forwards the resulting Ark VTXO to the supplied Ark address mailbox.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**lightning_invoice_for_address_request** | [**LightningInvoiceForAddressRequest**](LightningInvoiceForAddressRequest.md) |  | [required] |

### Return type

[**models::InvoiceInfo**](InvoiceInfo.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_receive_status

> models::LightningReceiveInfo get_receive_status(identifier)
Get receive status

Returns the status of a specified Lightning receive, identified by its payment hash, invoice string, or preimage. The response tracks progress through timestamps: `htlc_vtxos` is populated once HTLCs are created by the Ark server, `preimage_revealed_at` records when the preimage was sent, and `finished_at` indicates the receive has settled or been canceled.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**identifier** | **String** | Payment hash, invoice string or preimage to search for | [required] |

### Return type

[**models::LightningReceiveInfo**](LightningReceiveInfo.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_send_status

> models::LightningSendInfo get_send_status(identifier)
Get send status

Returns the status of a specified outgoing Lightning payment, identified by its payment hash or invoice string. The `state` field tracks the payment lifecycle from `start` through `paid` or `failed`; the preimage is included once the payment succeeded, and `failure_reason` once it failed. A failed payment stays failed: paying the same invoice again through this API is refused. If the wallet does not recognize the payment hash, it will return `unknown`. This is a read on the status in the db, so it does not trigger any `sync` before checking the state.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**identifier** | **String** | Payment hash or invoice string to search for | [required] |

### Return type

[**models::LightningSendInfo**](LightningSendInfo.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_receive_statuses

> Vec<models::LightningReceiveInfo> list_receive_statuses()
List all pending receive statuses

Returns the statuses of all pending Lightning receives, ordered from oldest to newest. A receive is pending until its `finished_at` timestamp is set, indicating it has settled or been canceled.

### Parameters

This endpoint does not need any parameter.

### Return type

[**Vec<models::LightningReceiveInfo>**](LightningReceiveInfo.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## pay

> models::LightningPayResponse pay(lightning_pay_request)
Send a Lightning payment

Sends a payment to a Lightning destination. Accepts a BOLT11 invoice, BOLT12 offer, or Lightning address. The `amount_sat` field is required for Lightning addresses but optional for invoices and offers. Comments are only supported for Lightning addresses. With `max_fee_sat`, the payment is refused before any funds are committed if its fee would be higher. Paying an invoice whose earlier payment failed is refused, so a failure stays final. Returns once the payment has been started, so its status can be polled right away.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**lightning_pay_request** | [**LightningPayRequest**](LightningPayRequest.md) |  | [required] |

### Return type

[**models::LightningPayResponse**](LightningPayResponse.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## settle_hold_receive

> serde_json::Value settle_hold_receive(payment_hash, lightning_hold_settle_request)
Settle a hold receive

Supplies the preimage of a hold receive in the `awaiting_preimage` state and claims the payment. The preimage is stored before it is revealed, so after an internal error the call can be repeated, and the wallet also finishes the claim on its own. Repeating the call after the payment was claimed succeeds.

### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**payment_hash** | **String** | Payment hash of the hold receive | [required] |
**lightning_hold_settle_request** | [**LightningHoldSettleRequest**](LightningHoldSettleRequest.md) |  | [required] |

### Return type

[**serde_json::Value**](serde_json::Value.md)

### Authorization

[bearer](../README.md#bearer)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

