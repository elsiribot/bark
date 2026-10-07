# LightningHoldInvoiceRequest

## Properties

Name | Type | Description | Notes
------------ | ------------- | ------------- | -------------
**amount_sat** | **i64** | The amount to create the invoice for (in satoshis). This is the amount the payee will pay but the final amount received by the client will have any server-configured [LightningReceiveFees](crate::cli::fees::LightningReceiveFees) deducted. | 
**description** | Option<**String**> | Optional description embedded in the invoice as its memo. | [optional]
**expiry_secs** | **i64** | For how many seconds from now a payment is accepted. A payment arriving later is not claimed. The invoice itself carries the Ark server's expiry. | 
**payment_hash** | **String** | The payment hash of the invoice as 64 hex characters. The caller holds its preimage and supplies it later to settle the receive. | 

[[Back to Model list]](../README.md#documentation-for-models) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to README]](../README.md)


