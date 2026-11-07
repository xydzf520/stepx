package com.stepx.companion.neo;
public final class CompanionMdiCoordinatesTest {
    public static void main(String[] args){
        if(!CompanionMdiCoordinates.visualEvidenceMatches("搜索我的订单","搜索 我的订单"))throw new AssertionError("OCR whitespace");
        if(CompanionMdiCoordinates.visualEvidenceMatches("我的订单","搜索我的订单")||CompanionMdiCoordinates.visualEvidenceMatches("订单","订单")||CompanionMdiCoordinates.visualEvidenceMatches("我的订单","我的收藏"))throw new AssertionError("weak or different visual evidence accepted");
        if(CompanionMdiCoordinates.pixel(500,1200)!=600||CompanionMdiCoordinates.pixel(1000,2670)!=2669)throw new AssertionError("scale/boundary");
        for(double n:new double[]{-1,1001,Double.NaN,Double.POSITIVE_INFINITY})try{CompanionMdiCoordinates.pixel(n,1200);throw new AssertionError("invalid coordinate accepted");}catch(IllegalArgumentException expected){}
        String t=CompanionMdiCoordinates.visibleTree("search [0,0][1200,267]\nvisible [600,1335][1200,2670]\noffscreen [0,3000][1200,3300]\nclipped [-10,2500][1400,2900]",1200,2670);
        if(!t.contains("search [0,0][1000,100]")||!t.contains("visible [500,500][1000,1000]")||t.contains("offscreen")||!t.contains("clipped [0,936][1000,1000]"))throw new AssertionError(t);
        System.out.println("MDI coordinates: normalized bounds, clipping and invalid action rejection passed");
        String before="root [0,0][1200,2670]\n 2 text='广告A' [0,0][1200,1000]\n 3 desc='我的' [1000,2400][1200,2670]";
        String after="root [0,0][1200,2670]\n 2 text='广告B' [0,0][1200,1000]\n 9 desc='我的' [1000,2400][1200,2670]";
        if(!CompanionMdiCoordinates.stableTarget(before,after,920,950,1200,2670))throw new AssertionError("unrelated ad must not block stable tab");
        if(CompanionMdiCoordinates.stableTarget(before,after.replace("我的","付款"),920,950,1200,2670))throw new AssertionError("changed target accepted");
        if(CompanionMdiCoordinates.stableTarget(before,after+"\noverlay [0,0][1200,2670]",920,950,1200,2670))throw new AssertionError("overlay accepted");
        if(CompanionMdiCoordinates.stableTarget(before,after.replace("[1000,2400]","[1000,2600]"),920,950,1200,2670))throw new AssertionError("moved target accepted");
    }
}
