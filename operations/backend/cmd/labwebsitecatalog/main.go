// labwebsitecatalog prepares local review fixtures through the real commercial
// snapshot and public projection code. It has no database, signing or publishing access.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/productcatalog"
)

type input struct {
	PlanID     string                `json:"plan_id"`
	Version    uint32                `json:"version"`
	Definition commercial.Definition `json:"definition"`
}

type output struct {
	Catalog commercial.PublicCatalog  `json:"catalog"`
	SHA256  string                    `json:"sha256"`
	Plans   []commercial.PlanSnapshot `json:"plans"`
}

func prepare(in input) (output, error) {
	if in.Definition.Offer.Kind != "free" {
		return output{}, fmt.Errorf("review input must identify the bundled free plan")
	}
	free, err := commercial.FreezePlan(in.PlanID, in.Version, in.Definition)
	if err != nil {
		return output{}, err
	}
	snapshot, err := free.Snapshot()
	if err != nil {
		return output{}, err
	}
	plans := []commercial.PlanSnapshot{snapshot}
	selection := []commercial.CatalogSelection{{PlanID: in.PlanID, Version: in.Version, ExpectedSHA256: free.Digest()}}
	for _, tier := range []struct {
		id, name string
		seats    uint32
		amount   int64
	}{
		{"local_review_20", "20 席位版", 20, 599900},
		{"local_review_50", "50 席位版", 50, 999900},
		{"local_review_custom", "更多席位", 50, 0},
	} {
		// These are review examples, never defaults for real orders. Unconfirmed
		// transfer/support policies are marked local and are not published.
		def := commercial.Definition{
			Product: productcatalog.Product, Code: tier.id, Name: tier.name,
			Description: "本地方案预览 非正式报价 安装指导 配置咨询与产品缺陷支持",
			Edition:     "local-review", MinimumVersion: in.Definition.MinimumVersion,
			QuotaPolicyVersion:  in.Definition.QuotaPolicyVersion,
			SupportTermsVersion: "local-review-support-pending", TransferLimit: 0,
			Entitlements: productcatalog.Entitlements{
				CatalogVersion: productcatalog.Version,
				Features:       []productcatalog.CapabilityID{},
				FeatureSets:    []productcatalog.FeatureSetID{productcatalog.FeatureSetStandard},
				Quotas: []productcatalog.QuotaGrant{
					{ID: productcatalog.QuotaMemberSeats, Limit: productcatalog.Limited(tier.seats)},
					{ID: productcatalog.QuotaRunners, Limit: productcatalog.Unlimited()},
					{ID: productcatalog.QuotaUpstreamAccounts, Limit: productcatalog.Unlimited()},
					{ID: productcatalog.QuotaApiKeysPerMember, Limit: productcatalog.Unlimited()},
				},
			},
			Offer: commercial.Offer{Kind: "annual", Currency: "CNY", AnnualAmountMinor: tier.amount,
				TaxMode: "none", TermRule: "calendar_years_clamp_day", TermTimezone: "Asia/Shanghai",
				Terms: []commercial.Term{{Years: 1, DiscountBasisPoints: 10000}, {Years: 2, DiscountBasisPoints: 9000},
					{Years: 3, DiscountBasisPoints: 8500}, {Years: 4, DiscountBasisPoints: 8000}, {Years: 5, DiscountBasisPoints: 7000}}},
		}
		if tier.amount == 0 {
			def.Offer = commercial.Offer{Kind: "contact"}
			def.Description = "超过 50 席位按实际需求报价 成交席位与价格另行确认 本地方案预览"
		}
		frozen, err := commercial.FreezePlan(tier.id, 1, def)
		if err != nil {
			return output{}, err
		}
		plan, err := frozen.Snapshot()
		if err != nil {
			return output{}, err
		}
		plans = append(plans, plan)
		selection = append(selection, commercial.CatalogSelection{PlanID: tier.id, Version: 1, ExpectedSHA256: frozen.Digest()})
	}
	request := commercial.CatalogRequest{OperationID: "local_website_review", Environment: "local",
		Reason: "Local review only; no Operations approval, order, license or publication", Plans: selection}
	bytes, err := request.Bytes()
	if err != nil {
		return output{}, err
	}
	preview, err := commercial.BuildPublicCatalog("catalog_"+commercial.ContentDigest(bytes)[:48], request, plans)
	if err != nil {
		return output{}, err
	}
	return output{Catalog: preview.Catalog, SHA256: preview.SHA256, Plans: plans}, nil
}

func execute(r io.Reader, w io.Writer) error {
	decoder := json.NewDecoder(io.LimitReader(r, 128*1024+1))
	decoder.DisallowUnknownFields()
	var in input
	if err := decoder.Decode(&in); err != nil {
		return err
	}
	var trailing any
	if err := decoder.Decode(&trailing); err != io.EOF {
		return fmt.Errorf("unexpected trailing input")
	}
	result, err := prepare(in)
	if err != nil {
		return err
	}
	return json.NewEncoder(w).Encode(result)
}

func main() {
	if err := execute(os.Stdin, os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
